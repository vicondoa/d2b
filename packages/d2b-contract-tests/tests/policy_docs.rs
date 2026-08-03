//! Docs / AGENTS / CLI-manpage / kernel-module-matrix policy lints (the
//! "H-group"), migrated from the `tests/*-eval.sh` bash gates. Each test reads
//! the real repo files (via the `d2b_contract_tests` repo-file helpers) and
//! asserts a documentation / source-parity invariant. This crate runs only from
//! `tests/tools/rust-workspace-checks.sh` against the real checkout (it is excluded
//! from the hermetic Nix sandbox workspace build), so repo-file access is sound.
//!
//! Migrated gates:
//!   * tests/agents-md-rewrite-eval.sh    -> agents_md_reflects_daemon_only_end_state
//!   * tests/manpage-completeness-eval.sh -> manpage_documents_every_top_level_subcommand
//!   * tests/kernel-module-matrix-eval.sh -> kernel_module_matrix_source_doc_parity
//!     + kernel_module_missing_typed_error_contract

use std::collections::{BTreeMap, BTreeSet};

use d2b_contract_tests::{read_repo_file, repo_path_exists};
use regex::Regex;
use serde_json::Value;

/// Whether any single line of `content` matches `pattern`. This mirrors `grep`'s
/// per-line evaluation faithfully (so a `\s*` in the pattern can never span a
/// newline boundary, as it could with a whole-file `Regex::is_match`).
fn any_line_matches(content: &str, pattern: &str) -> bool {
    let re = Regex::new(pattern).expect("valid regex");
    content.lines().any(|line| re.is_match(line))
}

// ---------------------------------------------------------------------------
// Migrated from tests/agents-md-rewrite-eval.sh.
//
// Asserts AGENTS.md reflects the daemon-only end-state (ADR 0015): no line may
// describe the bash CLI or a per-VM systemd template as a *live* framework
// surface. Historical / retired / "deleted in" context is allowed when the line
// is explicitly marked as such.
//
// Two halves, ported verbatim from the bash gate:
//   * Positive invariants - the rewrite must surface the daemon-only end-state
//     section explicitly, cross-reference ADR 0015, and mention d2bd /
//     d2b-priv-broker.socket / SpawnRunner.
//   * Negative invariants - a per-line scan: any line matching a forbidden
//     legacy-as-live pattern is a violation UNLESS the same line also carries an
//     explicit historical / retired marker (matched case-insensitively).
// ---------------------------------------------------------------------------
#[test]
fn agents_md_reflects_daemon_only_end_state() {
    let rel = "AGENTS.md";
    assert!(
        repo_path_exists(rel),
        "agents-md-rewrite-eval: missing {rel}"
    );
    let agents = read_repo_file(rel);

    // --- Positive invariants (grep -qE, per-line) -------------------------
    assert!(
        any_line_matches(&agents, r"^## Daemon-only end-state \(P6 onward\)"),
        "AGENTS.md is missing the '## Daemon-only end-state (P6 onward)' section"
    );
    assert!(
        any_line_matches(&agents, r"0015-daemon-only-clean-break\.md"),
        "AGENTS.md does not cross-reference docs/adr/0015-daemon-only-clean-break.md"
    );
    assert!(
        any_line_matches(&agents, r"d2bd"),
        "AGENTS.md does not mention d2bd"
    );
    assert!(
        any_line_matches(&agents, r"d2b-priv-broker\.socket"),
        "AGENTS.md does not mention d2b-priv-broker.socket (socket-activation contract)"
    );
    assert!(
        any_line_matches(&agents, r"SpawnRunner"),
        "AGENTS.md does not describe broker SpawnRunner for TPM/USBIP/GPU rewire"
    );

    // --- Negative invariants (per-line forbidden scan w/ allowed marker) --
    let violations = scan_retired_surfaces(&agents, rel);

    assert!(
        violations.is_empty(),
        "agents-md-rewrite-eval: {} line(s) describe retired surfaces as live; see ADR 0015:\n{}",
        violations.len(),
        violations.join("\n")
    );
}

/// Per-line scan for retired surfaces described as live.
///
/// `forbidden_re` (grep -nE, case-sensitive) and `allowed_marker_re`
/// (grep -qEi, case-insensitive) are ported verbatim from the bash gate.
/// The forbidden alternation targets the canonical legacy-as-live shapes:
/// d2b@<vm> / microvm@<vm> per-VM systemd templates, retired host-singleton
/// framework services, microvms.target, the legacy bash-CLI opt-in knobs, and
/// the "bash CLI" phrase. A line keeps its forbidden pattern only when it ALSO
/// mentions a historical / migration / retired marker (it is describing the
/// deletion itself).
fn scan_retired_surfaces(content: &str, label: &str) -> Vec<String> {
    let forbidden_re = Regex::new(
        r"d2b@<vm>|d2b@\$\{name\}|d2b@sys-|microvm@<vm>|microvm-virtiofsd@|microvm-set-booted@|microvm-tap-interfaces@|microvm-macvtap-interfaces@|microvm-pci-devices@|d2b-<vm>-(gpu|snd|video|swtpm|store-sync)\.service|d2b-sys-<env>-usbipd|d2b-otel-relay@|d2b-known-hosts-refresh@|d2b-vfsd-watchdog@|d2b-ch-exporter\.service|d2b-otel-host-bridge\.service|d2b-net-route-preflight\.service|d2b-audit-check\.(service|timer)|microvms\.target|D2B_LEGACY_BASH_OPT_IN|D2B_LEGACY_CLI|\bbash CLI\b",
    )
    .expect("valid forbidden regex");
    let allowed_marker_re = Regex::new(
        r"(?i)retired|removed|deleted|legacy|historical|no longer|no per-|no per-VM|end-state|P6|pre-v1|v0\.4|ADR 0015|denylist|ph6-|rewire|rewritten|migration|supersedes|reintroduce|Don't|There is no|not mention|moved into|either moved|fail-closed|-style",
    )
    .expect("valid allowed-marker regex");

    let mut violations: Vec<String> = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if !forbidden_re.is_match(line) {
            continue;
        }
        if allowed_marker_re.is_match(line) {
            continue;
        }
        violations.push(format!(
            "{label}:{} describes a retired surface as live (no historical marker): {line}",
            idx + 1
        ));
    }
    violations
}

/// The contributor process docs under `docs/contributing/` hold prose moved out
/// of AGENTS.md. Without this, the retired-surface scan above would keep
/// passing while no longer scanning the text it was written to police.
#[test]
fn contributing_docs_reflect_daemon_only_end_state() {
    let mut violations: Vec<String> = Vec::new();
    for rel in contributing_docs() {
        violations.extend(scan_retired_surfaces(&read_repo_file(&rel), &rel));
    }

    assert!(
        violations.is_empty(),
        "{} line(s) in docs/contributing/ describe retired surfaces as live; see ADR 0015:\n{}",
        violations.len(),
        violations.join("\n")
    );
}

const CONTRIBUTING_DIR: &str = "docs/contributing";

/// Why a directory listing could not be turned into a scan set. Both cases are
/// failures: a lint whose input set silently shrinks reports a clean scan of
/// the docs it managed to read.
#[derive(Debug, PartialEq, Eq)]
enum DirScanFault {
    /// A `read_dir` entry could not be read. An unreadable entry is a doc the
    /// scan did not look at, and a doc the scan did not look at is not a doc
    /// the scan cleared.
    UnreadableEntry { dir: String, detail: String },
    /// The directory holds no Markdown file at all, so the scan below it would
    /// pass by having nothing to scan.
    NoMarkdownFiles { dir: String },
}

/// The sorted repo-relative Markdown paths in one directory listing.
///
/// Taking `Result` items rather than a directory path is what makes the
/// fail-closed half testable: the entry error this must not discard cannot be
/// provoked from a real `read_dir` on demand. Non-Markdown entries are filtered
/// normally - they are not errors - but an `Err` entry ends the scan.
fn markdown_docs_in<I>(dir: &str, entries: I) -> Result<Vec<String>, DirScanFault>
where
    I: IntoIterator<Item = std::io::Result<String>>,
{
    let mut out: Vec<String> = Vec::new();
    for entry in entries {
        let name = entry.map_err(|err| DirScanFault::UnreadableEntry {
            dir: dir.to_string(),
            detail: err.to_string(),
        })?;
        if name.ends_with(".md") {
            out.push(format!("{dir}/{name}"));
        }
    }
    out.sort();
    if out.is_empty() {
        return Err(DirScanFault::NoMarkdownFiles {
            dir: dir.to_string(),
        });
    }
    Ok(out)
}

/// Repo-relative paths of every Markdown file under `docs/contributing/`.
fn contributing_docs() -> Vec<String> {
    let dir = d2b_contract_tests::repo_root().join(CONTRIBUTING_DIR);
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|err| {
            panic!("{CONTRIBUTING_DIR} must exist; AGENTS.md routes to it: {err}")
        })
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()));

    markdown_docs_in(CONTRIBUTING_DIR, entries).unwrap_or_else(|fault| {
        panic!(
            "{CONTRIBUTING_DIR} could not be enumerated, so the scans below it have not run: \
             {fault:?}"
        )
    })
}

#[test]
fn contributing_doc_enumeration_fails_closed_on_an_unreadable_entry() {
    let entries = |names: Vec<std::io::Result<String>>| markdown_docs_in(CONTRIBUTING_DIR, names);

    // Markdown is collected, sorted, and prefixed; other entries are filtered.
    assert_eq!(
        entries(vec![
            Ok("workflow.md".to_string()),
            Ok("README".to_string()),
            Ok("assets".to_string()),
            Ok("architecture.md".to_string()),
        ]),
        Ok(vec![
            format!("{CONTRIBUTING_DIR}/architecture.md"),
            format!("{CONTRIBUTING_DIR}/workflow.md"),
        ])
    );

    // The fail-open shape this replaced: `entry.ok()?` in a `filter_map` turned
    // the unreadable entry below into one fewer scanned doc and a green run.
    // The error is reported whether or not readable Markdown was also found,
    // and whether it arrives before or after that Markdown.
    for entries_with_error in [
        vec![
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "workflow.md: denied",
            )),
            Ok("architecture.md".to_string()),
        ],
        vec![
            Ok("architecture.md".to_string()),
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "workflow.md: denied",
            )),
        ],
    ] {
        assert_eq!(
            entries(entries_with_error),
            Err(DirScanFault::UnreadableEntry {
                dir: CONTRIBUTING_DIR.to_string(),
                detail: "workflow.md: denied".to_string(),
            }),
            "an unreadable directory entry must fail the scan, not shrink it"
        );
    }

    // A directory with no Markdown in it at all.
    assert_eq!(
        entries(vec![Ok("assets".to_string())]),
        Err(DirScanFault::NoMarkdownFiles {
            dir: CONTRIBUTING_DIR.to_string(),
        })
    );
    assert_eq!(
        entries(vec![]),
        Err(DirScanFault::NoMarkdownFiles {
            dir: CONTRIBUTING_DIR.to_string(),
        })
    );
}

/// AGENTS.md is injected into every agent session by every harness, so its size
/// is a fixed cost paid before any work begins. It reached 122,662 bytes by
/// accretion, because every change appended to it. This ratchet makes the next
/// re-bloating append fail instead: put the detail in `docs/contributing/` and
/// leave a rule and a link here.
///
/// Raising this budget is a deliberate decision, not a formality. Before you
/// do, check that the content genuinely belongs in the always-loaded index
/// rather than in a doc the agent opens when it needs it.
#[test]
fn agents_md_stays_within_its_context_budget() {
    const BUDGET: usize = 40_000;
    let bytes = read_repo_file("AGENTS.md").len();
    assert!(
        bytes <= BUDGET,
        "AGENTS.md is {bytes} bytes, over its {BUDGET}-byte budget. It is loaded into every \
         agent session on every turn. Move detail into docs/contributing/ and leave a rule \
         plus a link, rather than raising the budget."
    );
}

/// A router whose links rot is worse than the monolith it replaced: the rule
/// looks documented while the detail is unreachable.
#[test]
fn agents_md_routes_to_paths_that_exist() {
    let agents = read_repo_file("AGENTS.md");
    let link_re = Regex::new(r"\]\((\./[^)#]+)").expect("valid link regex");
    let mut missing: Vec<String> = Vec::new();
    for caps in link_re.captures_iter(&agents) {
        let rel = caps[1].trim_start_matches("./");
        if !repo_path_exists(rel) {
            missing.push(rel.to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "AGENTS.md links to {} path(s) that do not exist: {}",
        missing.len(),
        missing.join(", ")
    );
}

// ---------------------------------------------------------------------------
// Migrated from tests/manpage-completeness-eval.sh.
//
// Asserts that every top-level clap subcommand declared in
// `packages/d2b/src/lib.rs` (`enum NativeCommand { ... }`) is documented as
// a section in the committed d2b(1) manpage at `docs/manpages/d2b.1`.
// clap_mangen emits one `.TP` entry per subcommand under the SUBCOMMANDS block
// (rendered as `d2b-<name>(1)`); a new verb that lands without rerunning
// `cargo xtask gen-cli-shell-artifacts` silently drops out of the manpage. This
// gate fails closed on that drift without needing a cargo toolchain.
// ---------------------------------------------------------------------------
#[test]
fn manpage_documents_every_top_level_subcommand() {
    let cli_rel = "packages/d2b/src/lib.rs";
    let manpage_rel = "docs/manpages/d2b.1";
    assert!(
        repo_path_exists(cli_rel),
        "manpage-completeness: missing CLI source {cli_rel}"
    );
    assert!(
        repo_path_exists(manpage_rel),
        "manpage-completeness: missing manpage {manpage_rel}"
    );

    let expected = expected_subcommands(&read_repo_file(cli_rel));
    assert!(
        !expected.is_empty(),
        "manpage-completeness: failed to extract any subcommands from {cli_rel} (parser drift?)"
    );

    let documented = documented_subcommands(&read_repo_file(manpage_rel));
    assert!(
        !documented.is_empty(),
        "manpage-completeness: failed to extract any documented subcommands from {manpage_rel} \
         (manpage shape drift?)"
    );

    let missing: Vec<&String> = expected.difference(&documented).collect();
    assert!(
        missing.is_empty(),
        "manpage-completeness: subcommand(s) declared in {cli_rel} but missing from {manpage_rel} \
         (regenerate with: cargo xtask gen-cli-shell-artifacts):\n{}",
        missing
            .iter()
            .map(|m| format!("  - {m}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn ui_color_contract_docs_match_schema_surface() {
    let doc_rel = "docs/reference/ui-colors.md";
    let schema_rel = "docs/reference/ui-colors-schema.json";
    assert!(
        repo_path_exists(doc_rel),
        "ui-color-contract: missing {doc_rel}"
    );
    assert!(
        repo_path_exists(schema_rel),
        "ui-color-contract: missing {schema_rel}"
    );

    let doc = read_repo_file(doc_rel);
    let schema: Value =
        serde_json::from_str(&read_repo_file(schema_rel)).expect("ui color schema is valid JSON");

    assert_eq!(
        schema.get("$id").and_then(Value::as_str),
        Some("https://vicondoa.github.io/d2b/schemas/ui-colors-v1.json"),
        "ui color schema id drifted"
    );
    assert_eq!(
        schema
            .pointer("/properties/version/const")
            .and_then(Value::as_i64),
        Some(1),
        "ui color schema version drifted"
    );

    for required in [
        "version",
        "host",
        "states",
        "envs",
        "vms",
        "pendingRestart",
        "transitioning",
        "ui-colors-schema.json",
        "d2b_host_accent",
        "d2b_state_running",
        "d2b_env_<env>_accent",
        "d2b_vm_<vm>_border_active",
    ] {
        assert!(
            doc.contains(required),
            "ui color reference doc is missing required contract token: {required}"
        );
    }
}

#[test]
fn activation_docs_do_not_describe_host_side_guest_activation() {
    let cli = read_repo_file("docs/reference/cli-contract.md");
    let daemon_api = read_repo_file("docs/reference/daemon-api.md");
    let readme = read_repo_file("README.md");
    let design = read_repo_file("docs/explanation/design.md");

    for (rel, content) in [
        ("docs/reference/cli-contract.md", cli.as_str()),
        ("docs/reference/daemon-api.md", daemon_api.as_str()),
        ("README.md", readme.as_str()),
        ("docs/explanation/design.md", design.as_str()),
    ] {
        let lower = content.to_lowercase();
        for forbidden in [
            "broker directly executes switch-to-configuration",
            "broker runs switch-to-configuration",
            "runactivation executes switch-to-configuration",
            "host runs switch-to-configuration for the guest",
        ] {
            assert!(
                !lower.contains(forbidden),
                "{rel} claims the host/broker directly executes guest activation: {forbidden}"
            );
        }
    }

    for required in [
        "guestd to activate that prepared toplevel",
        "Stopped/offline VMs fail closed",
        "`boot --apply` is the explicit way to stage a new toplevel",
        "There is no host-side execution of guest activation scripts",
    ] {
        assert!(
            cli.contains(required),
            "cli-contract activation docs are missing required safe-activation wording: {required}"
        );
    }
    assert!(
        daemon_api
            .contains("Live activation (`Switch`, `Test`, and live `Rollback`) is not a broker"),
        "daemon-api must state live activation is not a broker script-execution surface"
    );
    assert!(
        readme.contains("guestd activates the prepared toplevel"),
        "README must explain that guestd activates prepared toplevels inside the VM"
    );
    assert!(
        design.contains("The broker never runs the guest's activation program"),
        "design overview must document the host-systemd isolation boundary"
    );
}

/// Faithful port of the bash gate's `awk` extraction of the `enum NativeCommand`
/// subcommand set. Two forms are recognised inside the enum block:
///   1. An explicit override `#[command(name = "...")]` on the line immediately
///      preceding a variant.
///   2. The default clap conversion: a `Ident(...)` variant whose PascalCase
///      identifier becomes kebab-case lowercase.
///
/// Only variants of the form `^<ws>Ident(` (a tuple-data variant) are detected,
/// exactly as the bash awk parser did.
fn expected_subcommands(cli_src: &str) -> BTreeSet<String> {
    let enum_start = Regex::new(r"^enum NativeCommand[[:space:]]*\{").unwrap();
    let enum_end = Regex::new(r"^\}").unwrap();
    let override_re =
        Regex::new(r#"^[[:space:]]*#\[command\(name[[:space:]]*=[[:space:]]*"[^"]+"\)\]"#).unwrap();
    let override_capture = Regex::new(r#""([^"]+)""#).unwrap();
    let variant_re = Regex::new(r"^[[:space:]]*[A-Z][A-Za-z0-9_]*\(").unwrap();
    let leading_ws = Regex::new(r"^[[:space:]]+").unwrap();

    let mut out: BTreeSet<String> = BTreeSet::new();
    let mut in_enum = false;
    let mut override_name: Option<String> = None;

    for line in cli_src.lines() {
        if enum_start.is_match(line) {
            in_enum = true;
            continue;
        }
        if in_enum && enum_end.is_match(line) {
            in_enum = false;
            continue;
        }
        if !in_enum {
            continue;
        }
        if override_re.is_match(line) {
            if let Some(cap) = override_capture.captures(line) {
                override_name = Some(cap[1].to_string());
            }
            continue;
        }
        if variant_re.is_match(line) {
            if let Some(name) = override_name.take() {
                out.insert(name);
                continue;
            }
            // Strip leading whitespace + trailing "(...".
            let stripped = leading_ws.replace(line, "");
            let ident = match stripped.find('(') {
                Some(pos) => &stripped[..pos],
                None => &stripped,
            };
            out.insert(pascal_to_kebab(ident));
        }
    }
    out
}

/// PascalCase → kebab-case lowercase, matching the bash awk per-character loop
/// (an uppercase char at index > 0 is prefixed with `-`, every char lowercased).
fn pascal_to_kebab(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if i > 0 && ch.is_ascii_uppercase() {
            out.push('-');
        }
        out.extend(ch.to_lowercase());
    }
    out
}

/// Faithful port of the bash gate's `awk` extraction of the documented
/// subcommands from the SUBCOMMANDS block of the rendered manpage. Lines under
/// `.SH SUBCOMMANDS` (until the next `.SH `) that start with the roff-escaped
/// `d2b\-` prefix are reduced to their bare `<name>` by stripping the
/// prefix + `(1)` suffix and un-escaping `\-` back to `-`.
fn documented_subcommands(manpage: &str) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    let mut in_sub = false;
    for line in manpage.lines() {
        if line == ".SH SUBCOMMANDS" {
            in_sub = true;
            continue;
        }
        if in_sub && line.starts_with(".SH ") {
            in_sub = false;
            continue;
        }
        if in_sub && let Some(rest) = line.strip_prefix("d2b\\-") {
            let rest = rest.strip_suffix("(1)").unwrap_or(rest);
            out.insert(rest.replace("\\-", "-"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Migrated from tests/kernel-module-matrix-eval.sh (matrix-parity half).
//
// Asserts the REQUIRED / OPTIONAL module constants in
// `packages/d2bd/src/kernel_module_check.rs` stay in sync with the
// operator-reference matrix in `docs/reference/kernel-module-check.md`. The
// source must carry each canonical `"<module>"` string literal, the doc must
// cite each module backticked, and the source must declare each canonical
// `pub const <IDENT>` (so a stealth refactor that renames a constant surfaces).
// ---------------------------------------------------------------------------
#[test]
fn kernel_module_matrix_source_doc_parity() {
    let src_rel = "packages/d2bd/src/kernel_module_check.rs";
    let doc_rel = "docs/reference/kernel-module-check.md";
    assert!(
        repo_path_exists(src_rel),
        "kernel-module-matrix-eval: source not found: {src_rel}"
    );
    assert!(
        repo_path_exists(doc_rel),
        "kernel-module-matrix-eval: operator reference not found: {doc_rel}"
    );
    let src = read_repo_file(src_rel);
    let doc = read_repo_file(doc_rel);

    let required_always = [
        "vhost_net",
        "tun",
        "virtio_net",
        "virtio_blk",
        "virtio_pci",
        "virtio_console",
    ];
    let required_kvm = ["kvm_intel", "kvm_amd"];
    let required_virtiofs = "virtiofs";
    let required_graphics = ["udmabuf", "drm_virtgpu"];
    let optional_nvidia = ["nvidia", "nvidia_uvm"];
    let optional_usbip = "usbip_host";
    let optional_tpm = "tpm_vtpm_proxy";

    // Source-side assertions: every module name appears quoted (grep -qF "\"$m\"").
    let mut src_modules: Vec<&str> = Vec::new();
    src_modules.extend(required_always);
    src_modules.extend(required_kvm);
    src_modules.push(required_virtiofs);
    src_modules.extend(required_graphics);
    src_modules.extend(optional_nvidia);
    src_modules.push(optional_usbip);
    src_modules.push(optional_tpm);
    for m in &src_modules {
        assert!(
            src.contains(&format!("\"{m}\"")),
            "kernel-module-matrix-eval: missing '\"{m}\"' in kernel_module_check.rs"
        );
    }

    // Doc-side assertions: the operator reference cites every module backticked.
    for m in &src_modules {
        assert!(
            doc.contains(&format!("`{m}`")),
            "kernel-module-matrix-eval: missing backticked '`{m}`' in kernel-module-check.md"
        );
    }

    // Source must EXACTLY name the canonical public constants.
    for ident in [
        "REQUIRED_ALWAYS",
        "REQUIRED_KVM_ALTERNATIVES",
        "REQUIRED_IF_VIRTIOFS",
        "REQUIRED_IF_GRAPHICS",
        "OPTIONAL_GRAPHICS_NVIDIA",
        "OPTIONAL_USBIP",
        "OPTIONAL_TPM",
    ] {
        assert!(
            any_line_matches(&src, &format!("pub const {ident}")),
            "kernel-module-matrix-eval: src missing public constant: {ident}"
        );
    }
}

// ---------------------------------------------------------------------------
// USB security-key docs scaffolding existence gate.
//
// Asserts that the docs scaffolding files for the USB security-key proxy
// feature are present in the repo. This is a policy gate - it ensures the
// docs/test surface does not silently disappear in a partial revert and that
// the implementation workstream has a concrete target to make green.
//
// Checked files:
//   * docs/how-to/use-usb-security-key.md
//   * docs/how-to/migrate-usbip-yubikey-to-security-key.md
//   * docs/reference/components-usb-security-key.md
//   * docs/reference/usb-security-key-events.md
//   * docs/explanation/usb-security-key-architecture.md
//   * tests/unit/nix/cases/usb-security-key.nix
//   * tests/golden/cli-output/usb-security-key-help.txt
//   * tests/golden/cli-output/usb-security-key-status-help.txt
//   * tests/golden/cli-output/usb-security-key-sessions-help.txt
//   * tests/golden/cli-output/usb-security-key-cancel-help.txt
//   * tests/golden/cli-output/usb-security-key-test-help.txt
// ---------------------------------------------------------------------------
#[test]
fn usb_security_key_docs_scaffolding_present() {
    let required = [
        "docs/how-to/use-usb-security-key.md",
        "docs/how-to/migrate-usbip-yubikey-to-security-key.md",
        "docs/reference/components-usb-security-key.md",
        "docs/reference/usb-security-key-events.md",
        "docs/explanation/usb-security-key-architecture.md",
        "tests/unit/nix/cases/usb-security-key.nix",
        "tests/golden/cli-output/usb-security-key-help.txt",
        "tests/golden/cli-output/usb-security-key-status-help.txt",
        "tests/golden/cli-output/usb-security-key-sessions-help.txt",
        "tests/golden/cli-output/usb-security-key-cancel-help.txt",
        "tests/golden/cli-output/usb-security-key-test-help.txt",
    ];
    for rel in &required {
        assert!(
            repo_path_exists(rel),
            "usb-security-key-docs-scaffolding: missing expected file: {rel}"
        );
    }
}

// ---------------------------------------------------------------------------
// No process/autopilot markers in USB security-key docs.
//
// Asserts that the shipped security-key docs do not contain autopilot
// process-pipeline markers (wave IDs, phase codes, fleet-execution artefacts,
// or forbidden OS names). These must not appear in operator-visible docs.
// ---------------------------------------------------------------------------
#[test]
fn usb_security_key_docs_no_process_markers() {
    let doc_files = [
        "docs/how-to/use-usb-security-key.md",
        "docs/how-to/migrate-usbip-yubikey-to-security-key.md",
        "docs/reference/components-usb-security-key.md",
        "docs/reference/usb-security-key-events.md",
        "docs/explanation/usb-security-key-architecture.md",
    ];

    // Forbidden patterns: autopilot/wave/fleet process markers and
    // forbidden OS names that must not appear in user-facing docs.
    let forbidden_patterns = [
        "W3fu",
        "ForbiddenLiveOSName",
        "autopilot_marker",
        "WAVE_ID",
        "PHASE_MARKER",
        "fleet_execution",
    ];

    let mut violations: Vec<String> = Vec::new();
    for rel in &doc_files {
        if !repo_path_exists(rel) {
            continue; // existence is checked by the scaffolding gate above
        }
        let content = read_repo_file(rel);
        for pattern in &forbidden_patterns {
            for (idx, line) in content.lines().enumerate() {
                if line.contains(pattern) {
                    violations.push(format!(
                        "{rel}:{}: process marker '{pattern}' must not appear in shipped docs: {line}",
                        idx + 1
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "usb-security-key-docs-no-process-markers: {} violation(s):\n{}",
        violations.len(),
        violations.join("\n")
    );
}

// ---------------------------------------------------------------------------
// USB security-key CLI golden files are non-empty stubs.
//
// The CLI goldens are placeholder stubs committed alongside the docs. Each
// must be a non-empty file (at minimum one line of expected help text)
// so the contract test does not silently pass against an empty file after a
// partial revert. The implementation workstream replaces the stubs with the
// real `d2b usb security-key …` output once the CLI is implemented.
// ---------------------------------------------------------------------------
#[test]
fn usb_security_key_cli_goldens_are_non_empty() {
    let golden_files = [
        "tests/golden/cli-output/usb-security-key-help.txt",
        "tests/golden/cli-output/usb-security-key-status-help.txt",
        "tests/golden/cli-output/usb-security-key-sessions-help.txt",
        "tests/golden/cli-output/usb-security-key-cancel-help.txt",
        "tests/golden/cli-output/usb-security-key-test-help.txt",
    ];
    for rel in &golden_files {
        if !repo_path_exists(rel) {
            continue; // existence checked by scaffolding gate
        }
        let content = read_repo_file(rel);
        assert!(
            !content.trim().is_empty(),
            "usb-security-key-cli-goldens-non-empty: {rel} is empty; stubs must contain placeholder help text"
        );
        assert!(
            content.contains("security-key") || content.contains("security key"),
            "usb-security-key-cli-goldens-non-empty: {rel} does not mention 'security-key' or 'security key'; content:\n{content}"
        );
    }
}

// ---------------------------------------------------------------------------
// Migrated from tests/kernel-module-matrix-eval.sh (typed-error contract half).
//
// Asserts the fatal-typed-error contract: `packages/d2bd/src/typed_error.rs`
// carries the `HostKernelModulesMissing` variant at exit code 64 with kind
// "host-kernel-modules-missing".
// ---------------------------------------------------------------------------
#[test]
fn kernel_module_missing_typed_error_contract() {
    let typed_rel = "packages/d2bd/src/typed_error.rs";
    assert!(
        repo_path_exists(typed_rel),
        "kernel-module-matrix-eval: typed_error.rs not found: {typed_rel}"
    );
    let typed = read_repo_file(typed_rel);

    assert!(
        typed.contains("HostKernelModulesMissing"),
        "kernel-module-matrix-eval: typed_error missing HostKernelModulesMissing variant"
    );
    assert!(
        typed.contains("\"host-kernel-modules-missing\""),
        "kernel-module-matrix-eval: typed_error missing kind 'host-kernel-modules-missing'"
    );
    assert!(
        any_line_matches(&typed, r"HostKernelModulesMissing \{ \.\. \} => 64"),
        "kernel-module-matrix-eval: typed_error missing exit code 64 for HostKernelModulesMissing"
    );
}

// ---------------------------------------------------------------------------
// Test execution manifest schema/prose agreement.
//
// This is intentionally a policy lint rather than a fixture-backed contract:
// a schema or reference-doc edit must not silently make the producer and
// operator guidance disagree. The positive path discovers the required
// top-level fields from the binding schema. The mutated prose below is the
// negative fixture proving that a version drift is rejected.
// ---------------------------------------------------------------------------
fn execution_manifest_schema_prose_versions_agree(schema: &Value, prose: &str) -> bool {
    let schema_version = schema
        .pointer("/properties/version/const")
        .and_then(Value::as_i64);
    let prose_version = Regex::new(r"The binding schema version is \*\*([0-9]+)\*\*")
        .expect("valid execution-manifest version regex")
        .captures(prose)
        .and_then(|captures| captures.get(1))
        .and_then(|capture| capture.as_str().parse::<i64>().ok());
    schema_version.is_some() && schema_version == prose_version
}

#[test]
fn execution_manifest_schema_and_prose_agree_with_non_empty_discovery() {
    let schema_rel = "docs/reference/schemas/test-execution-manifest-v1.json";
    let prose_rel = "docs/reference/test-execution-manifest.md";
    let helper_rel = "tests/tools/execution-manifest.pl";
    assert!(
        repo_path_exists(schema_rel),
        "execution-manifest-policy: missing {schema_rel}"
    );
    assert!(
        repo_path_exists(prose_rel),
        "execution-manifest-policy: missing {prose_rel}"
    );
    assert!(
        repo_path_exists(helper_rel),
        "execution-manifest-policy: missing {helper_rel}"
    );

    let schema: Value =
        serde_json::from_str(&read_repo_file(schema_rel)).expect("execution manifest schema JSON");
    let prose = read_repo_file(prose_rel);
    let helper = read_repo_file(helper_rel);
    let makefile = read_repo_file("Makefile");
    let rust_driver = read_repo_file("tests/test-rust.sh");
    let api_driver = read_repo_file("tests/tools/api-surface-json.sh");
    assert_eq!(
        schema.get("$id").and_then(Value::as_str),
        Some("https://vicondoa.github.io/d2b/schemas/test-execution-manifest-v1.json"),
        "execution-manifest-policy: schema id drifted"
    );
    assert!(
        execution_manifest_schema_prose_versions_agree(&schema, &prose),
        "execution-manifest-policy: schema and prose versions disagree"
    );

    let properties = schema
        .pointer("/properties")
        .and_then(Value::as_object)
        .expect("execution-manifest schema properties");
    assert!(
        !properties.is_empty(),
        "execution-manifest-policy: field discovery is empty"
    );
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .expect("execution-manifest schema required fields");
    assert!(
        !required.is_empty(),
        "execution-manifest-policy: required-field discovery is empty"
    );
    for marker in [
        "O_CLOEXEC",
        "O_NOFOLLOW",
        "F_OFD_SETLK",
        "openat",
        "unlinkat",
    ] {
        assert!(
            helper.contains(marker),
            "execution-manifest-policy: helper is missing secure lifecycle marker {marker}"
        );
    }
    assert!(
        !helper.contains("rm -rf"),
        "execution-manifest-policy: helper must not use path-based recursive cleanup"
    );
    for field in required {
        let field = field
            .as_str()
            .expect("execution-manifest required field is a string");
        assert!(
            properties.contains_key(field),
            "execution-manifest-policy: required field {field} is not declared"
        );
        assert!(
            prose.contains(&format!("`{field}`")),
            "execution-manifest-policy: prose does not document required field {field}"
        );
    }

    // Negative mutation fixture: a version edit in prose must fail closed.
    let mutated = prose.replacen(
        "The binding schema version is **1**.",
        "The binding schema version is **2**.",
        1,
    );
    assert_ne!(
        mutated, prose,
        "execution-manifest-policy: negative mutation fixture did not mutate prose"
    );
    assert!(
        !execution_manifest_schema_prose_versions_agree(&schema, &mutated),
        "execution-manifest-policy: version mutation was not rejected"
    );

    let rust_baseline_leaves = [
        "rust-api-surface",
        "rust-main-format",
        "rust-main-clippy",
        "rust-main-workspace-tests",
        "rust-contract-tests",
        "rust-cli-contract-tests",
        "rust-no-bash-ast",
        "rust-broker-default",
        "rust-broker-layer1",
        "rust-broker-fakebackends",
        "rust-guest-shell-runner",
        "rust-schema-reproducibility",
        "rust-deny-main",
        "rust-deny-broker",
        "rust-deny-guest",
        "rust-audit-main",
        "rust-audit-broker",
        "rust-audit-guest",
        "rust-stub-no-socket",
        "rust-assert-pinned",
    ];
    let schema_rust_leaves = schema
        .pointer("/allOf/0/then/properties/completed_leaves/items/enum")
        .and_then(Value::as_array)
        .expect("execution-manifest-policy: Rust baseline enum is missing");
    for leaf in rust_baseline_leaves {
        assert!(
            rust_driver.contains(leaf),
            "execution-manifest-policy: Rust emitter does not name baseline leaf {leaf}"
        );
        assert!(
            schema_rust_leaves
                .iter()
                .any(|value| value.as_str() == Some(leaf)),
            "execution-manifest-policy: schema does not allow baseline leaf {leaf}"
        );
        assert!(
            prose.contains(&format!("`{leaf}`")),
            "execution-manifest-policy: reference prose does not document baseline leaf {leaf}"
        );
    }
    assert!(
        !makefile.contains("test-rust-leaf-fixture-contracts: test-rust-leaf-main-workspace"),
        "execution-manifest-policy: isolated fixture target regained the main-workspace edge"
    );
    assert!(
        rust_driver
            .contains("fixture_target_dir=\"$ROOT/.scratch/rust-test-cache/fixture-contracts\"")
            && rust_driver.contains("fixture_target_dir=\"$workspace_target_dir\"")
            && rust_driver.contains("${D2B_RUST_COLD_PROFILE:-0}"),
        "execution-manifest-policy: fixture warm/shared target selection drifted"
    );
    assert!(
        api_driver.contains("public_target=\"$target_root/public-census\"")
            && api_driver.contains("private_target=\"$target_root/private-census\"")
            && api_driver.contains("public_target=\"$target_root/census\"")
            && api_driver.contains("shared_census=1")
            && api_driver.contains("checker_target=\"$target_root/checker\"")
            && api_driver.contains("CARGO_BUILD_JOBS=\"$public_jobs\"")
            && api_driver.contains("CARGO_BUILD_JOBS=\"$private_jobs\"")
            && api_driver.contains(
                "CARGO_TARGET_DIR=\"$checker_target\" cargo run --quiet --release --locked"
            ),
        "execution-manifest-policy: API census targets or split quotas drifted"
    );
    assert!(
        makefile.contains("D2B_RUST_BROKER_PREREQS_aggregate := test-rust-leaf-inventory")
            && makefile.contains("test-rust-leaf-broker: $(D2B_RUST_BROKER_PREREQS)"),
        "execution-manifest-policy: broker target must wait for inventory before lockfile enumeration"
    );
    assert!(
        makefile.contains("D2B_SKIP_FIXTURE_BUILD"),
        "execution-manifest-policy: Rust aggregate lost the conditional fixture skip"
    );
    assert!(
        makefile.contains(
            "test-rust-main:\n\t+@$(call D2B_RUST_DISPATCH,$(D2B_RUST_MAIN_LEAVES),main)"
        ),
        "execution-manifest-policy: focused main target lost conditional fixture coverage"
    );
    let emitter_region = rust_driver
        .split("publish_manifest_fragment()")
        .nth(1)
        .and_then(|region| region.split("rust_surface_start()").next())
        .expect("execution-manifest-policy: fragment emitter helper is missing");
    assert!(
        !emitter_region.contains(">/dev/null"),
        "execution-manifest-policy: fragment publication errors must remain visible"
    );
    assert!(
        !emitter_region.contains("|| true"),
        "execution-manifest-policy: fragment publication must not be made best-effort on success"
    );
    assert!(
        rust_driver.contains("required execution-manifest fragment publication failed"),
        "execution-manifest-policy: successful-surface emitter failures lack a static diagnostic"
    );
    assert!(
        helper.contains("return 74")
            && helper.contains("finalization failed after scheduler success")
            && helper.contains("preserving the scheduler status"),
        "execution-manifest-policy: finalization errors do not distinguish scheduler status"
    );
}

// ---------------------------------------------------------------------------
// Panel-preflight doc / Makefile truth table.
//
// The contributing doc tells a contributor how to run the panel preflight. The
// spelling it gives has to be the spelling that exists: an earlier revision
// documented `make panel-preflight` before any such target was written, and
// nothing was looking. This lint looks.
//
// It reads two inputs - whether the `Makefile` declares a `panel-preflight`
// target, and the marked command / notice blocks in
// `docs/contributing/copilot-agents.md` - and admits exactly two pairings:
//
//   | State       | Makefile target | Operator command                        | Future notice |
//   |-------------|-----------------|-----------------------------------------|---------------|
//   | Current     | absent          | `node scripts/copilot/check-bindings.mjs` | present     |
//   | Implemented | present         | `make panel-preflight`                  | absent        |
//
// Every other pairing is a mixed state and is rejected with a distinguishable
// reason. The lint fails closed on a missing, duplicated, reversed or empty
// marker block and on an input it cannot read, because a consistency check
// that cannot locate both sides has not shown consistency.
// ---------------------------------------------------------------------------

const PANEL_PREFLIGHT_TARGET: &str = "panel-preflight";
const PANEL_PREFLIGHT_COMMAND_MARKER: &str = "PANEL-PREFLIGHT-COMMAND";
const PANEL_PREFLIGHT_NOTICE_MARKER: &str = "PANEL-PREFLIGHT-NOTICE";
const PANEL_PREFLIGHT_NODE_COMMAND: &str = "node scripts/copilot/check-bindings.mjs";
const PANEL_PREFLIGHT_MAKE_COMMAND: &str = "make panel-preflight";
const PANEL_PREFLIGHT_DOC: &str = "docs/contributing/copilot-agents.md";

/// Why a `<!-- BEGIN X -->` / `<!-- END X -->` pair could not be read as one
/// block. Every case is a failure: a lint that silently treats a missing or
/// duplicated marker as "nothing to check" checks nothing.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum MarkerFault {
    BeginMissing,
    EndMissing,
    DuplicateBegin,
    DuplicateEnd,
    /// `END` precedes `BEGIN`.
    Reversed,
}

/// What the notice block says about the target's existence.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum NoticeState {
    /// The markers are balanced and hold no prose: the notice was removed, as
    /// the implementing commit is told to do.
    Absent,
    /// The notice still claims the target does not exist yet.
    FutureNotImplemented,
    /// A notice that makes some other claim - the implemented-state
    /// replacement. Admitted wherever `Absent` is admitted.
    NonFuture,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum PanelPreflightState {
    /// Target absent, node command documented, future notice present.
    CurrentNodeCommand,
    /// Target present, `make panel-preflight` documented, future notice gone.
    ImplementedMakeTarget,
}

#[derive(Debug, PartialEq, Eq)]
enum PanelPreflightReject {
    MakefileUnreadable,
    DocUnreadable,
    CommandMarker(MarkerFault),
    NoticeMarker(MarkerFault),
    /// A code fence inside the command block is never closed.
    MalformedCommandBlock,
    /// The command block gives no command at all (empty, or prose only).
    EmptyCommandBlock,
    /// The command block gives a command, but neither recognised spelling.
    NoRecognisedOperatorCommand,
    /// The exact state a panel round caught in this repository's history.
    TargetAbsentButMakeCommandGiven,
    /// The doc no longer says the target is missing, and it is still missing.
    TargetAbsentButFutureNoticeMissing(NoticeState),
    /// Half the preflight goes unrun: the target shipped and the doc still
    /// points at the checker it wraps.
    TargetPresentButNodeCommandGiven,
    /// The doc tells contributors a shipped target does not exist.
    TargetPresentButFutureNoticePresent,
}

/// The lines strictly between `<!-- BEGIN {marker} -->` and
/// `<!-- END {marker} -->`.
fn extract_marked_block(doc: &str, marker: &str) -> Result<Vec<String>, MarkerFault> {
    let begin = format!("<!-- BEGIN {marker} -->");
    let end = format!("<!-- END {marker} -->");
    let mut begin_idx: Option<usize> = None;
    let mut end_idx: Option<usize> = None;
    for (idx, line) in doc.lines().enumerate() {
        let line = line.trim();
        if line == begin {
            if begin_idx.is_some() {
                return Err(MarkerFault::DuplicateBegin);
            }
            begin_idx = Some(idx);
        } else if line == end {
            if end_idx.is_some() {
                return Err(MarkerFault::DuplicateEnd);
            }
            end_idx = Some(idx);
        }
    }
    let begin_idx = begin_idx.ok_or(MarkerFault::BeginMissing)?;
    let end_idx = end_idx.ok_or(MarkerFault::EndMissing)?;
    if end_idx <= begin_idx {
        return Err(MarkerFault::Reversed);
    }
    Ok(doc
        .lines()
        .skip(begin_idx + 1)
        .take(end_idx - begin_idx - 1)
        .map(str::to_string)
        .collect())
}

/// Whether the `Makefile` *declares* `target` as a rule target, rather than
/// merely mentioning it.
///
/// Make rules are line-oriented: a rule line starts in column zero, names its
/// targets before the first `:`, and is not a variable assignment. So a
/// `.PHONY: panel-preflight` line, a `foo: panel-preflight` prerequisite, a
/// recipe line, a comment, and a `panel-preflight-args := …` assignment all
/// mention the name without declaring the target - and a substring search
/// would call every one of them an implementation.
fn makefile_declares_target(makefile: &str, target: &str) -> bool {
    makefile.lines().any(|line| {
        if line.starts_with([' ', '\t']) || line.trim_start().starts_with('#') {
            return false;
        }
        let Some(colon) = line.find(':') else {
            return false;
        };
        let rest = &line[colon + 1..];
        // `name := value`, `name ::= value`: an assignment, not a rule.
        if rest.starts_with('=') || rest.starts_with(":=") {
            return false;
        }
        line[..colon].split_whitespace().any(|name| name == target)
    })
}

/// The commands a marked block *gives an operator*: the non-empty,
/// non-comment lines inside its fenced code blocks. Prose around the fence -
/// an "under the hood this runs …" note, or a debugging aid - is deliberately
/// not an operator command, which is what lets the implemented state keep
/// naming the node checker without re-entering the current state.
fn operator_commands(block: &[String]) -> Result<Vec<String>, PanelPreflightReject> {
    let mut out: Vec<String> = Vec::new();
    let mut in_fence = false;
    for line in block {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            continue;
        }
        let command = trimmed.trim_start_matches("$ ").trim();
        if command.is_empty() || command.starts_with('#') {
            continue;
        }
        out.push(command.to_string());
    }
    if in_fence {
        return Err(PanelPreflightReject::MalformedCommandBlock);
    }
    Ok(out)
}

/// Classify the notice block. The future notice is recognised by the claim it
/// makes - that the target does not exist yet - not by its exact wording, so a
/// reflow does not silently turn it into a non-future notice.
fn notice_state(block: &[String]) -> NoticeState {
    let text = block
        .iter()
        .map(|line| line.trim().trim_start_matches('>').trim().to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    if text.trim().is_empty() {
        return NoticeState::Absent;
    }
    let claims_future = [
        "not yet implemented",
        "not implemented yet",
        "does not exist yet",
        "until it lands",
    ]
    .iter()
    .any(|phrase| text.contains(phrase));
    if claims_future {
        NoticeState::FutureNotImplemented
    } else {
        NoticeState::NonFuture
    }
}

/// The whole truth table, as a pure function of the two inputs. `None` models
/// an input the caller could not read; it is a rejection, never a skip.
fn evaluate_panel_preflight(
    makefile: Option<&str>,
    doc: Option<&str>,
) -> Result<PanelPreflightState, PanelPreflightReject> {
    let makefile = makefile.ok_or(PanelPreflightReject::MakefileUnreadable)?;
    let doc = doc.ok_or(PanelPreflightReject::DocUnreadable)?;

    let command_block = extract_marked_block(doc, PANEL_PREFLIGHT_COMMAND_MARKER)
        .map_err(PanelPreflightReject::CommandMarker)?;
    let notice_block = extract_marked_block(doc, PANEL_PREFLIGHT_NOTICE_MARKER)
        .map_err(PanelPreflightReject::NoticeMarker)?;

    let commands = operator_commands(&command_block)?;
    if commands.is_empty() {
        return Err(PanelPreflightReject::EmptyCommandBlock);
    }
    let gives_make = commands
        .iter()
        .any(|command| command.contains(PANEL_PREFLIGHT_MAKE_COMMAND));
    let gives_node = commands
        .iter()
        .any(|command| command.contains(PANEL_PREFLIGHT_NODE_COMMAND));
    let notice = notice_state(&notice_block);

    if makefile_declares_target(makefile, PANEL_PREFLIGHT_TARGET) {
        if gives_node {
            return Err(PanelPreflightReject::TargetPresentButNodeCommandGiven);
        }
        if !gives_make {
            return Err(PanelPreflightReject::NoRecognisedOperatorCommand);
        }
        if notice == NoticeState::FutureNotImplemented {
            return Err(PanelPreflightReject::TargetPresentButFutureNoticePresent);
        }
        Ok(PanelPreflightState::ImplementedMakeTarget)
    } else {
        if gives_make {
            return Err(PanelPreflightReject::TargetAbsentButMakeCommandGiven);
        }
        if !gives_node {
            return Err(PanelPreflightReject::NoRecognisedOperatorCommand);
        }
        if notice != NoticeState::FutureNotImplemented {
            return Err(PanelPreflightReject::TargetAbsentButFutureNoticeMissing(
                notice,
            ));
        }
        Ok(PanelPreflightState::CurrentNodeCommand)
    }
}

/// The live tree: the real `Makefile` and the real contributing doc must sit in
/// one of the two admitted rows, not in any mix of them.
#[test]
fn panel_preflight_doc_matches_the_makefile() {
    assert!(
        repo_path_exists(PANEL_PREFLIGHT_DOC),
        "panel-preflight-policy: missing {PANEL_PREFLIGHT_DOC}"
    );
    let makefile = read_repo_file("Makefile");
    let doc = read_repo_file(PANEL_PREFLIGHT_DOC);

    let state = evaluate_panel_preflight(Some(&makefile), Some(&doc)).unwrap_or_else(|reject| {
        panic!(
            "panel-preflight-policy: the Makefile and {PANEL_PREFLIGHT_DOC} are in a mixed \
             state: {reject:?}. The documented operator command, the future notice, and the \
             presence of a `{PANEL_PREFLIGHT_TARGET}` target move together, in one commit."
        )
    });

    assert_eq!(
        state,
        PanelPreflightState::CurrentNodeCommand,
        "panel-preflight-policy: the tree has moved to the implemented row. That is the \
         intended destination, not a failure: change this expectation to \
         `PanelPreflightState::ImplementedMakeTarget` in the same commit that adds the \
         `{PANEL_PREFLIGHT_TARGET}` target, rewrites the command block, and drops the future \
         notice."
    );
}

// --- in-memory fixtures for the truth table --------------------------------

const NODE_COMMAND_BLOCK: &str = "```\nnode scripts/copilot/check-bindings.mjs\n```";
const MAKE_COMMAND_BLOCK: &str = "```\nmake panel-preflight\n```";
/// The implemented-state command block that still names the node checker, as
/// the underlying implementation, outside the fence. This must be admitted:
/// the rule is about what the operator is told to run, not about the string.
const MAKE_COMMAND_BLOCK_WITH_NODE_PROSE: &str = "```\nmake panel-preflight\n```\n\nUnder the \
     hood that runs `node scripts/copilot/check-bindings.mjs` plus the receipt resolver; run \
     the node checker directly only when debugging the bindings themselves.";
const FUTURE_NOTICE: &str = "> **Future, not yet implemented.**\n> A single `make \
     panel-preflight` target is proposed. That target does not exist yet. Until it lands, run \
     the node command above.";
const IMPLEMENTED_NOTICE: &str = "> `make panel-preflight` runs the binding checker, the harness receipt resolver, and the \
     version check.";

const MAKEFILE_WITH_TARGET: &str = "\
.PHONY: check panel-preflight

check: panel-preflight
\t@echo checked

panel-preflight:
\tnode scripts/copilot/check-bindings.mjs
";

/// Mentions `panel-preflight` four ways - a `.PHONY` list, a prerequisite, a
/// comment, and a variable assignment whose name has it as a prefix - and
/// declares it none of them.
const MAKEFILE_WITHOUT_TARGET: &str = "\
.PHONY: check panel-preflight

# panel-preflight: proposed, see ADR 0053.
panel-preflight-args := --strict

check: panel-preflight
\t@echo checked
";

fn panel_doc_fixture(command_block: &str, notice_block: &str) -> String {
    format!(
        "## Running check-bindings\n\
         \n\
         <!-- BEGIN {PANEL_PREFLIGHT_COMMAND_MARKER} -->\n\
         {command_block}\n\
         <!-- END {PANEL_PREFLIGHT_COMMAND_MARKER} -->\n\
         \n\
         It fails on an agent with no binding row.\n\
         \n\
         <!-- BEGIN {PANEL_PREFLIGHT_NOTICE_MARKER} -->\n\
         {notice_block}\n\
         <!-- END {PANEL_PREFLIGHT_NOTICE_MARKER} -->\n\
         \n\
         ## Panel seats\n"
    )
}

#[test]
fn panel_preflight_makefile_target_detection_is_line_oriented() {
    assert!(
        makefile_declares_target(MAKEFILE_WITH_TARGET, PANEL_PREFLIGHT_TARGET),
        "a rule line declaring the target must be detected"
    );
    assert!(
        !makefile_declares_target(MAKEFILE_WITHOUT_TARGET, PANEL_PREFLIGHT_TARGET),
        ".PHONY entries, prerequisites, comments and prefixed variables are mentions, not \
         declarations"
    );
    assert!(
        makefile_declares_target(
            "panel-preflight other-check: deps\n\t@echo hi\n",
            PANEL_PREFLIGHT_TARGET
        ),
        "a multi-target rule line declares each of its targets"
    );
    assert!(
        makefile_declares_target("panel-preflight::\n\t@echo hi\n", PANEL_PREFLIGHT_TARGET),
        "a double-colon rule declares its target"
    );
    assert!(
        !makefile_declares_target("panel-preflight := node x.mjs\n", PANEL_PREFLIGHT_TARGET),
        "a variable assignment is not a rule"
    );
    assert!(
        !makefile_declares_target("\tpanel-preflight: not a rule\n", PANEL_PREFLIGHT_TARGET),
        "a recipe line is not a rule line"
    );
}

#[test]
fn panel_preflight_admits_the_current_state() {
    let doc = panel_doc_fixture(NODE_COMMAND_BLOCK, FUTURE_NOTICE);
    assert_eq!(
        evaluate_panel_preflight(Some(MAKEFILE_WITHOUT_TARGET), Some(&doc)),
        Ok(PanelPreflightState::CurrentNodeCommand)
    );
}

#[test]
fn panel_preflight_admits_the_implemented_state() {
    for notice in [IMPLEMENTED_NOTICE, "", "   "] {
        let doc = panel_doc_fixture(MAKE_COMMAND_BLOCK, notice);
        assert_eq!(
            evaluate_panel_preflight(Some(MAKEFILE_WITH_TARGET), Some(&doc)),
            Ok(PanelPreflightState::ImplementedMakeTarget),
            "the implemented state admits a removed notice and a non-future replacement"
        );
    }

    let doc = panel_doc_fixture(MAKE_COMMAND_BLOCK_WITH_NODE_PROSE, IMPLEMENTED_NOTICE);
    assert_eq!(
        evaluate_panel_preflight(Some(MAKEFILE_WITH_TARGET), Some(&doc)),
        Ok(PanelPreflightState::ImplementedMakeTarget),
        "naming the node checker as the underlying implementation, outside the fence, is not \
         an operator instruction"
    );
}

#[test]
fn panel_preflight_rejects_every_mixed_state() {
    // The state a panel round caught here: the doc promoted a target that had
    // not been written, and left the notice saying so.
    assert_eq!(
        evaluate_panel_preflight(
            Some(MAKEFILE_WITHOUT_TARGET),
            Some(&panel_doc_fixture(MAKE_COMMAND_BLOCK, FUTURE_NOTICE))
        ),
        Err(PanelPreflightReject::TargetAbsentButMakeCommandGiven)
    );
    // Same missing target, notice dropped: the doc now points at nothing.
    assert_eq!(
        evaluate_panel_preflight(
            Some(MAKEFILE_WITHOUT_TARGET),
            Some(&panel_doc_fixture(NODE_COMMAND_BLOCK, ""))
        ),
        Err(PanelPreflightReject::TargetAbsentButFutureNoticeMissing(
            NoticeState::Absent
        ))
    );
    assert_eq!(
        evaluate_panel_preflight(
            Some(MAKEFILE_WITHOUT_TARGET),
            Some(&panel_doc_fixture(NODE_COMMAND_BLOCK, IMPLEMENTED_NOTICE))
        ),
        Err(PanelPreflightReject::TargetAbsentButFutureNoticeMissing(
            NoticeState::NonFuture
        ))
    );
    // Target shipped, doc still sends operators at the checker it wraps.
    assert_eq!(
        evaluate_panel_preflight(
            Some(MAKEFILE_WITH_TARGET),
            Some(&panel_doc_fixture(NODE_COMMAND_BLOCK, IMPLEMENTED_NOTICE))
        ),
        Err(PanelPreflightReject::TargetPresentButNodeCommandGiven)
    );
    // Target shipped, notice still says it does not exist.
    assert_eq!(
        evaluate_panel_preflight(
            Some(MAKEFILE_WITH_TARGET),
            Some(&panel_doc_fixture(MAKE_COMMAND_BLOCK, FUTURE_NOTICE))
        ),
        Err(PanelPreflightReject::TargetPresentButFutureNoticePresent)
    );
    // Both spellings given as operator commands, in either row.
    assert_eq!(
        evaluate_panel_preflight(
            Some(MAKEFILE_WITHOUT_TARGET),
            Some(&panel_doc_fixture(
                "```\nnode scripts/copilot/check-bindings.mjs\nmake panel-preflight\n```",
                FUTURE_NOTICE
            ))
        ),
        Err(PanelPreflightReject::TargetAbsentButMakeCommandGiven)
    );
    assert_eq!(
        evaluate_panel_preflight(
            Some(MAKEFILE_WITH_TARGET),
            Some(&panel_doc_fixture(
                "```\nnode scripts/copilot/check-bindings.mjs\nmake panel-preflight\n```",
                IMPLEMENTED_NOTICE
            ))
        ),
        Err(PanelPreflightReject::TargetPresentButNodeCommandGiven)
    );
    // A third spelling nobody recognises is not a pass in either row.
    for makefile in [MAKEFILE_WITHOUT_TARGET, MAKEFILE_WITH_TARGET] {
        assert_eq!(
            evaluate_panel_preflight(
                Some(makefile),
                Some(&panel_doc_fixture(
                    "```\nbash scripts/copilot/preflight.sh\n```",
                    FUTURE_NOTICE
                ))
            ),
            Err(PanelPreflightReject::NoRecognisedOperatorCommand)
        );
    }
}

#[test]
fn panel_preflight_marker_faults_fail_closed() {
    let command_begin = format!("<!-- BEGIN {PANEL_PREFLIGHT_COMMAND_MARKER} -->");
    let command_end = format!("<!-- END {PANEL_PREFLIGHT_COMMAND_MARKER} -->");
    let notice_end = format!("<!-- END {PANEL_PREFLIGHT_NOTICE_MARKER} -->");
    let valid = panel_doc_fixture(NODE_COMMAND_BLOCK, FUTURE_NOTICE);

    let cases: Vec<(String, PanelPreflightReject)> = vec![
        (
            valid.replace(&command_begin, ""),
            PanelPreflightReject::CommandMarker(MarkerFault::BeginMissing),
        ),
        (
            valid.replace(&command_end, ""),
            PanelPreflightReject::CommandMarker(MarkerFault::EndMissing),
        ),
        (
            valid.replace(&command_begin, &format!("{command_begin}\n{command_begin}")),
            PanelPreflightReject::CommandMarker(MarkerFault::DuplicateBegin),
        ),
        (
            valid.replace(&command_end, &format!("{command_end}\n{command_end}")),
            PanelPreflightReject::CommandMarker(MarkerFault::DuplicateEnd),
        ),
        (
            format!("{command_end}\n{NODE_COMMAND_BLOCK}\n{command_begin}\n"),
            PanelPreflightReject::CommandMarker(MarkerFault::Reversed),
        ),
        (
            valid.replace(&notice_end, ""),
            PanelPreflightReject::NoticeMarker(MarkerFault::EndMissing),
        ),
        (
            panel_doc_fixture("", FUTURE_NOTICE),
            PanelPreflightReject::EmptyCommandBlock,
        ),
        (
            panel_doc_fixture("Run the checker somehow.", FUTURE_NOTICE),
            PanelPreflightReject::EmptyCommandBlock,
        ),
        (
            panel_doc_fixture("```\n# just a comment\n```", FUTURE_NOTICE),
            PanelPreflightReject::EmptyCommandBlock,
        ),
        (
            panel_doc_fixture(
                "```\nnode scripts/copilot/check-bindings.mjs",
                FUTURE_NOTICE,
            ),
            PanelPreflightReject::MalformedCommandBlock,
        ),
    ];

    for (doc, expected) in cases {
        assert_eq!(
            evaluate_panel_preflight(Some(MAKEFILE_WITHOUT_TARGET), Some(&doc)),
            Err(expected),
            "marker fault fixture was not rejected as expected; doc was:\n{doc}"
        );
    }
}

#[test]
fn panel_preflight_unreadable_inputs_fail_closed() {
    let doc = panel_doc_fixture(NODE_COMMAND_BLOCK, FUTURE_NOTICE);
    assert_eq!(
        evaluate_panel_preflight(None, Some(&doc)),
        Err(PanelPreflightReject::MakefileUnreadable),
        "an unreadable Makefile is a rejection, not a skipped half of the check"
    );
    assert_eq!(
        evaluate_panel_preflight(Some(MAKEFILE_WITHOUT_TARGET), None),
        Err(PanelPreflightReject::DocUnreadable)
    );
    assert_eq!(
        evaluate_panel_preflight(None, None),
        Err(PanelPreflightReject::MakefileUnreadable)
    );
}

// ---------------------------------------------------------------------------
// Safe-type census predicate + planted fixtures.
//
// The panel receipt error enum does not exist in this repository yet, so
// nothing below inspects a shipped type and nothing below is evidence about
// one. What lands now is the *algorithm*, the declared root set it walks, and
// the corpus that proves both work: an exhaustive recursive census over a type
// graph, plus planted fixtures that it must accept and reject today.
//
// The census is **multi-root, not reachability-based**. The governed surface is
// a declared list someone maintains, not one type and whatever hangs off it:
// `remedies` computes a `RemedyPlan` rather than storing one, so no field edge
// leads from the panel receipt error to `RemedyPlan`, `RemedyAction` or
// `ProducerContext`. A census rooted only at the error enum would walk a small
// tree, report success, and never look at the types that carry the
// operator-facing content. Every governed type is a root in its own right, so
// adding a governed type means adding a root; a governed type nobody declared
// is simply not censused, which puts the blind spot in a list someone
// maintains rather than in the traversal's own semantics.
//
// The rule the census enforces: from every declared root, every field of every
// struct, and every variant and every variant-field of every enum, at any
// depth, must resolve to a member of the closed approved set - a redacting
// newtype, a closed enum whose own variant-fields satisfy the same rule, a
// bounded numeric, a version or stage newtype, or a collection of safe items.
// An empty root set, a root that resolves to no governable structure at all,
// raw text, a path, an unresolved type, and a cycle it cannot traverse to a
// fixed point are all failures. Enums are not leaves: a census that stops at
// variant names, or that descends one level into structs only, is the census
// that misses the field that leaks.
//
// **Wiring contract for the implementation commit.** The corpus below is a
// hand-written model, deliberately shaped like the metadata a real census has:
// named types, fields, variants, variant-fields, and type references resolved
// through one map. The commit that introduces the real panel receipt error
// enum and remedy types builds a `TypeCorpus` from those types' actual
// metadata, adds them to the declared root set, and calls
// `census_governed_types` on it - the same predicate, not a second one - and
// keeps the fixtures below as its negative corpus. Until then this file
// asserts a property of the predicate, not of any production type.
// ---------------------------------------------------------------------------

/// A type with no members, as the census sees it. The approved kinds are the
/// closed safe set; the rejected kinds are the raw types a leak arrives in,
/// modelled explicitly so a planted raw `String` is distinguishable from a
/// type the census simply could not resolve.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum LeafKind {
    /// A newtype whose `Debug` renders a fixed redaction placeholder.
    RedactingNewtype,
    /// A bounded count, duration, or other bounded number.
    BoundedNumeric,
    /// A parsed, bounded version newtype.
    VersionNewtype,
    /// A closed pipeline-stage newtype.
    StageNewtype,
    /// `String`, `OsString`, or an arbitrary text map: never approved.
    RawText,
    /// `Path` or `PathBuf`: never approved.
    RawPath,
}

impl LeafKind {
    fn approved(self) -> bool {
        matches!(
            self,
            Self::RedactingNewtype
                | Self::BoundedNumeric
                | Self::VersionNewtype
                | Self::StageNewtype
        )
    }
}

/// `(member name, referenced type name)` pairs - struct fields, or the fields
/// of one enum variant.
type FieldList = Vec<(&'static str, &'static str)>;

#[derive(Debug, Clone)]
enum TypeDef {
    Leaf(LeafKind),
    Struct(FieldList),
    /// `(variant name, variant fields)`. A fieldless variant carries an empty
    /// field list and is still counted.
    Enum(Vec<(&'static str, FieldList)>),
    /// A homogeneous collection (`Vec<T>`, or a map from a closed key to `T`):
    /// safe exactly when its item type is.
    Collection(&'static str),
}

/// The resolvable universe. A type reference that is not a key here is
/// unresolved, and unresolved is a failure rather than a pass.
type TypeCorpus = BTreeMap<&'static str, TypeDef>;

/// The variants and fields one type definition contributes to a count. A type
/// contributes them once, however many places reference it, so a diamond does
/// not inflate what the census claims to have examined.
fn structure_counts(def: &TypeDef) -> (usize, usize) {
    match def {
        TypeDef::Enum(variants) => (
            variants.len(),
            variants.iter().map(|(_, fields)| fields.len()).sum(),
        ),
        TypeDef::Struct(fields) => (0, fields.len()),
        TypeDef::Leaf(_) | TypeDef::Collection(_) => (0, 0),
    }
}

/// What one declared root's own type tree contained. Each root is walked
/// independently, so the numbers do not depend on the order the roots were
/// declared in, and a root whose whole tree was already examined from an
/// earlier root is still shown to have been resolved, entered and traversed.
#[derive(Debug, PartialEq, Eq)]
struct RootCoverage {
    root: &'static str,
    /// Every type examined from this root, the root itself included.
    types: BTreeSet<&'static str>,
    variants: usize,
    fields: usize,
}

impl RootCoverage {
    /// `(types, variants, fields)` - the shape the assertions below compare.
    fn counts(&self) -> (usize, usize, usize) {
        (self.types.len(), self.variants, self.fields)
    }
}

/// What the census examined. Reported, not asserted away: a census that cannot
/// say how much it looked at cannot be shown to have looked at anything. The
/// totals count each distinct type once across every root, so they are the size
/// of the governed union rather than the sum of overlapping trees; per-root
/// coverage is kept alongside them because a total alone cannot show that every
/// declared root was actually entered.
#[derive(Debug, PartialEq, Eq)]
struct CensusReport {
    /// One entry per declared root, in declaration order.
    roots: Vec<RootCoverage>,
    types: usize,
    variants: usize,
    fields: usize,
}

impl CensusReport {
    fn coverage(&self, root: &str) -> &RootCoverage {
        self.roots
            .iter()
            .find(|coverage| coverage.root == root)
            .expect("the census reports coverage for every declared root")
    }

    /// `(types, variants, fields)` across the governed union.
    fn counts(&self) -> (usize, usize, usize) {
        (self.types, self.variants, self.fields)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CensusReject {
    /// No declared roots. Governance is a maintained list, and a list that
    /// governs nothing is a declaration bug rather than a clean census.
    NoRoots,
    /// Nothing to traverse at all.
    EmptyCorpus,
    /// A declared root resolved but its whole tree exposes no variant and no
    /// field, so censusing it proved nothing. A root is a governed payload
    /// type: a bare leaf newtype, or a struct or enum with no members, is a
    /// declaration bug. The rule is per root, not per census, so one empty root
    /// cannot hide behind the fields another root contributed.
    RootExaminedNothing { root: String },
    /// A referenced type the census cannot resolve - a declared root included.
    /// Not a skip.
    Unresolved { type_name: String, path: String },
    UnapprovedLeaf {
        kind: LeafKind,
        type_name: String,
        path: String,
    },
    /// A reference back into a type still on the traversal stack. The census
    /// does not attempt a fixed point, so this is an unsupported shape.
    UnsupportedCycle { type_name: String, path: String },
}

/// The traversal of one declared root.
struct RootWalk<'a> {
    corpus: &'a TypeCorpus,
    /// Doubles as the memo: a type lands here once it is fully traversed, so a
    /// shared (diamond) reference is supported and counted once.
    coverage: RootCoverage,
    /// Types currently being traversed, innermost last.
    stack: Vec<&'static str>,
}

impl RootWalk<'_> {
    fn visit(&mut self, type_name: &'static str, path: &str) -> Result<(), CensusReject> {
        if self.stack.contains(&type_name) {
            return Err(CensusReject::UnsupportedCycle {
                type_name: type_name.to_string(),
                path: path.to_string(),
            });
        }
        if self.coverage.types.contains(type_name) {
            return Ok(());
        }
        let corpus = self.corpus;
        let Some(def) = corpus.get(type_name) else {
            return Err(CensusReject::Unresolved {
                type_name: type_name.to_string(),
                path: path.to_string(),
            });
        };

        self.stack.push(type_name);
        let (variants, fields) = structure_counts(def);
        self.coverage.variants += variants;
        self.coverage.fields += fields;
        match def {
            TypeDef::Leaf(kind) => {
                if !kind.approved() {
                    return Err(CensusReject::UnapprovedLeaf {
                        kind: *kind,
                        type_name: type_name.to_string(),
                        path: path.to_string(),
                    });
                }
            }
            TypeDef::Struct(fields) => {
                for &(field, field_type) in fields {
                    self.visit(field_type, &format!("{path}.{field}"))?;
                }
            }
            TypeDef::Enum(variants) => {
                for (variant, fields) in variants {
                    for &(field, field_type) in fields {
                        self.visit(field_type, &format!("{path}::{variant}.{field}"))?;
                    }
                }
            }
            TypeDef::Collection(item) => {
                self.visit(item, &format!("{path}[]"))?;
            }
        }
        self.stack.pop();
        self.coverage.types.insert(type_name);
        Ok(())
    }
}

/// Census every declared governed root and prove every member of every one of
/// their type trees is safe.
///
/// `roots` is the governed list, not a reachability frontier. Reachability is
/// not governance: a computed payload is governed and reaches nothing from the
/// type it is computed from, which is why an empty root set and a root that
/// cannot be resolved both fail closed instead of reporting a vacuous pass.
///
/// This is a fixture predicate, not a production scan: it reads the modelled
/// `corpus` it is handed. The implementation commit that adds the real panel
/// receipt error enum and remedy types is expected to build that corpus from
/// those types' own metadata and add them to the declared root set, calling
/// this function rather than writing a second census.
fn census_governed_types(
    corpus: &TypeCorpus,
    roots: &[&'static str],
) -> Result<CensusReport, CensusReject> {
    if roots.is_empty() {
        return Err(CensusReject::NoRoots);
    }
    if corpus.is_empty() {
        return Err(CensusReject::EmptyCorpus);
    }

    let mut covered = Vec::with_capacity(roots.len());
    for &root in roots {
        let mut walk = RootWalk {
            corpus,
            coverage: RootCoverage {
                root,
                types: BTreeSet::new(),
                variants: 0,
                fields: 0,
            },
            stack: Vec::new(),
        };
        walk.visit(root, root)?;
        if walk.coverage.variants == 0 && walk.coverage.fields == 0 {
            return Err(CensusReject::RootExaminedNothing {
                root: root.to_string(),
            });
        }
        covered.push(walk.coverage);
    }

    // The totals dedupe the overlap between roots: every governed type
    // contributes its variants and fields once, however many roots reach it.
    let governed: BTreeSet<&'static str> = covered
        .iter()
        .flat_map(|coverage| coverage.types.iter().copied())
        .collect();
    let (mut variants, mut fields) = (0, 0);
    for type_name in &governed {
        let def = corpus
            .get(type_name)
            .expect("every examined type resolved during its root's walk");
        let (root_variants, root_fields) = structure_counts(def);
        variants += root_variants;
        fields += root_fields;
    }

    Ok(CensusReport {
        roots: covered,
        types: governed.len(),
        variants,
        fields,
    })
}

/// The panel receipt error enum. A root like any other, and the type whose
/// *absence* of a stored remedy field is the reason the remedy types have to be
/// roots of their own.
const ERROR_ROOT: &str = "PanelReceiptError";

/// The declared, closed root set. Adding a governed type means adding it here;
/// a governed type that is not listed is not censused at all.
const CENSUS_ROOTS: &[&str] = &[ERROR_ROOT, "RemedyPlan", "ProducerContext", "RemedyAction"];

/// The accepted fixture: four governed roots whose type trees are entirely
/// approved. Between them they exercise every supported shape - variant fields,
/// a fieldless variant, a nested struct, a closed fieldless enum, a collection
/// of typed items whose own variant-fields are safe, and types reached from
/// more than one place (`HarnessVersion` and `PipelineStage` are each reached
/// twice inside the `RemedyPlan` tree, and three roots overlap).
///
/// The error enum deliberately stores **no** `RemedyPlan`: a remedy is computed
/// from an error, not carried by one. That is exactly why `RemedyPlan`,
/// `ProducerContext` and `RemedyAction` are declared roots - no field edge
/// reaches them from `PanelReceiptError`, so a reachability census rooted at
/// the error enum would never see them.
fn safe_panel_receipt_corpus() -> TypeCorpus {
    TypeCorpus::from([
        (
            "PanelReceiptError",
            TypeDef::Enum(vec![
                (
                    "HarnessVersionUnparseable",
                    vec![("observed", "BannerDigest"), ("stage", "ReceiptStage")],
                ),
                ("HarnessUnavailable", vec![("attempts", "AttemptCount")]),
                ("ReceiptRejected", vec![("alias", "CorrelationAlias")]),
                ("Cancelled", vec![]),
            ]),
        ),
        (
            "ReceiptStage",
            TypeDef::Enum(vec![
                ("Preflight", vec![]),
                ("Resolve", vec![]),
                ("Publish", vec![]),
            ]),
        ),
        (
            "RemedyPlan",
            TypeDef::Struct(vec![
                ("actions", "RemedyActionList"),
                ("harness", "HarnessVersion"),
                ("producer", "ProducerContext"),
            ]),
        ),
        ("RemedyActionList", TypeDef::Collection("RemedyAction")),
        (
            "RemedyAction",
            TypeDef::Enum(vec![
                ("RerunPreflight", vec![("stage", "PipelineStage")]),
                ("UpgradeHarness", vec![("required", "HarnessVersion")]),
                ("AskOperator", vec![("alias", "CorrelationAlias")]),
                ("Abort", vec![]),
            ]),
        ),
        (
            "ProducerContext",
            TypeDef::Struct(vec![
                ("stage", "PipelineStage"),
                ("seat", "SeatDigest"),
                ("attempt", "AttemptCount"),
            ]),
        ),
        ("BannerDigest", TypeDef::Leaf(LeafKind::RedactingNewtype)),
        ("SeatDigest", TypeDef::Leaf(LeafKind::RedactingNewtype)),
        (
            "CorrelationAlias",
            TypeDef::Leaf(LeafKind::RedactingNewtype),
        ),
        ("AttemptCount", TypeDef::Leaf(LeafKind::BoundedNumeric)),
        ("HarnessVersion", TypeDef::Leaf(LeafKind::VersionNewtype)),
        ("PipelineStage", TypeDef::Leaf(LeafKind::StageNewtype)),
    ])
}

/// The safe corpus with `overrides` applied - the planted-fixture builder.
/// Each negative below changes exactly one thing, so the reason the census
/// rejects it is the thing that was planted.
fn corpus_with(overrides: &[(&'static str, TypeDef)]) -> TypeCorpus {
    let mut corpus = safe_panel_receipt_corpus();
    for (name, def) in overrides {
        corpus.insert(*name, def.clone());
    }
    corpus
}

/// The entry enum with one extra variant appended, for planting a defect at the
/// top level of the error root's own tree.
fn root_enum_with_extra_variant(variant: &'static str, fields: FieldList) -> TypeDef {
    let TypeDef::Enum(mut variants) = safe_panel_receipt_corpus()
        .get(ERROR_ROOT)
        .expect("the fixture corpus defines its error root")
        .clone()
    else {
        panic!("the fixture error root is an enum");
    };
    variants.push((variant, fields));
    TypeDef::Enum(variants)
}

#[test]
fn safe_type_census_accepts_the_approved_fixture() {
    let report = census_governed_types(&safe_panel_receipt_corpus(), CENSUS_ROOTS)
        .expect("the approved fixture corpus is accepted");

    // Per-root coverage as `(types, variants, fields)`: every declared root was
    // resolved, entered and walked. These type counts sum to 22 against 12
    // distinct governed types, and that overlap is what the totals dedupe.
    assert_eq!(
        report
            .roots
            .iter()
            .map(|coverage| (coverage.root, coverage.counts()))
            .collect::<Vec<_>>(),
        vec![
            (ERROR_ROOT, (5, 7, 4)),
            ("RemedyPlan", (9, 4, 9)),
            ("ProducerContext", (4, 0, 3)),
            ("RemedyAction", (4, 4, 3)),
        ],
        "each declared root must be walked on its own account; per-root drift means a root stopped being censused"
    );

    // Non-vacuous totals over the governed union: 12 distinct types, 11
    // variants (4 on the error enum, 3 on the closed fieldless enum, 4 on
    // `RemedyAction`) and 13 fields, each counted once however many roots
    // reach it.
    assert_eq!(
        report.counts(),
        (12, 11, 13),
        "the census must report what it examined; a drifting count means the traversal changed"
    );
}

#[test]
fn approved_fixture_computes_remedies_rather_than_storing_them() {
    let report = census_governed_types(&safe_panel_receipt_corpus(), CENSUS_ROOTS)
        .expect("the approved fixture corpus is accepted");
    let error = report.coverage(ERROR_ROOT);

    // The error enum carries no remedy field, so no field edge reaches the
    // remedy types from it. That is the invariant *and* the reason governance
    // cannot be reachability: each of these is censused because it is declared,
    // not because something points at it.
    for governed in [
        "RemedyPlan",
        "RemedyActionList",
        "RemedyAction",
        "ProducerContext",
    ] {
        assert!(
            !error.types.contains(governed),
            "no field edge may run from {ERROR_ROOT} to {governed}: remedies are computed, not stored"
        );
        assert!(
            report
                .roots
                .iter()
                .any(|coverage| coverage.types.contains(governed)),
            "{governed} is unreachable from the error enum, so a declared root must still cover it"
        );
    }
}

#[test]
fn safe_type_census_rejects_a_violation_only_the_remedy_plan_root_reaches() {
    // The plant is a field of `RemedyPlan` itself, so it sits in exactly one
    // root's tree: not the error enum's (nothing reaches `RemedyPlan` from it),
    // not `ProducerContext`'s, and not `RemedyAction`'s.
    let corpus = corpus_with(&[
        (
            "RemedyPlan",
            TypeDef::Struct(vec![
                ("actions", "RemedyActionList"),
                ("harness", "HarnessVersion"),
                ("producer", "ProducerContext"),
                ("summary", "RawMessage"),
            ]),
        ),
        ("RawMessage", TypeDef::Leaf(LeafKind::RawText)),
    ]);

    assert_eq!(
        census_governed_types(&corpus, CENSUS_ROOTS),
        Err(CensusReject::UnapprovedLeaf {
            kind: LeafKind::RawText,
            type_name: "RawMessage".to_string(),
            path: "RemedyPlan.summary".to_string(),
        })
    );

    // The discriminating half: the same corpus is *accepted* when the census is
    // rooted only at the error enum, and when it is rooted at the two other
    // governed types. A single-root or reachability-based implementation passes
    // every other fixture in this file and still ships this leak.
    assert_eq!(
        census_governed_types(&corpus, &[ERROR_ROOT]).map(|report| report.counts()),
        Ok((5, 7, 4)),
        "a census rooted at the error enum cannot see the remedy tree at all"
    );
    assert!(
        census_governed_types(&corpus, &["ProducerContext", "RemedyAction"]).is_ok(),
        "the plant is in neither of those trees; only RemedyPlan's own root catches it"
    );
}

#[test]
fn safe_type_census_rejects_planted_unsafe_members() {
    // A raw String at the top level of the error root, carrying no protected
    // marking at all.
    assert_eq!(
        census_governed_types(
            &corpus_with(&[
                (
                    ERROR_ROOT,
                    root_enum_with_extra_variant("Diagnostic", vec![("message", "RawMessage")])
                ),
                ("RawMessage", TypeDef::Leaf(LeafKind::RawText)),
            ]),
            CENSUS_ROOTS
        ),
        Err(CensusReject::UnapprovedLeaf {
            kind: LeafKind::RawText,
            type_name: "RawMessage".to_string(),
            path: "PanelReceiptError::Diagnostic.message".to_string(),
        })
    );

    // A PathBuf at the top level of the error root.
    assert_eq!(
        census_governed_types(
            &corpus_with(&[
                (
                    ERROR_ROOT,
                    root_enum_with_extra_variant("ReceiptMissing", vec![("path", "ReceiptPath")])
                ),
                ("ReceiptPath", TypeDef::Leaf(LeafKind::RawPath)),
            ]),
            CENSUS_ROOTS
        ),
        Err(CensusReject::UnapprovedLeaf {
            kind: LeafKind::RawPath,
            type_name: "ReceiptPath".to_string(),
            path: "PanelReceiptError::ReceiptMissing.path".to_string(),
        })
    );

    // A raw String on a struct field two levels below a root
    // (`RemedyPlan` -> `ProducerContext` -> field). A census that inspects only
    // the level below each root passes this fixture.
    assert_eq!(
        census_governed_types(
            &corpus_with(&[
                (
                    "ProducerContext",
                    TypeDef::Struct(vec![
                        ("stage", "PipelineStage"),
                        ("seat", "SeatDigest"),
                        ("note", "RawMessage"),
                    ])
                ),
                ("RawMessage", TypeDef::Leaf(LeafKind::RawText)),
            ]),
            CENSUS_ROOTS
        ),
        Err(CensusReject::UnapprovedLeaf {
            kind: LeafKind::RawText,
            type_name: "RawMessage".to_string(),
            path: "RemedyPlan.producer.note".to_string(),
        })
    );

    // A raw String on an enum variant-field two levels below a root, reached
    // through another enum. A census that stops at variant names passes this
    // fixture.
    assert_eq!(
        census_governed_types(
            &corpus_with(&[
                (
                    ERROR_ROOT,
                    root_enum_with_extra_variant("Escalated", vec![("inner", "InnerError")])
                ),
                (
                    "InnerError",
                    TypeDef::Enum(vec![
                        ("Transient", vec![("attempts", "AttemptCount")]),
                        ("Detail", vec![("evidence", "RawMessage")]),
                    ])
                ),
                ("RawMessage", TypeDef::Leaf(LeafKind::RawText)),
            ]),
            CENSUS_ROOTS
        ),
        Err(CensusReject::UnapprovedLeaf {
            kind: LeafKind::RawText,
            type_name: "RawMessage".to_string(),
            path: "PanelReceiptError::Escalated.inner::Detail.evidence".to_string(),
        })
    );

    // A path on a variant-field of the typed action collection:
    // `RemedyPlan` -> collection -> `RemedyAction` variant.
    assert_eq!(
        census_governed_types(
            &corpus_with(&[
                (
                    "RemedyAction",
                    TypeDef::Enum(vec![
                        (
                            "RerunPreflight",
                            vec![("stage", "PipelineStage"), ("workdir", "ReceiptPath")]
                        ),
                        ("Abort", vec![]),
                    ])
                ),
                ("ReceiptPath", TypeDef::Leaf(LeafKind::RawPath)),
            ]),
            CENSUS_ROOTS
        ),
        Err(CensusReject::UnapprovedLeaf {
            kind: LeafKind::RawPath,
            type_name: "ReceiptPath".to_string(),
            path: "RemedyPlan.actions[]::RerunPreflight.workdir".to_string(),
        })
    );
}

#[test]
fn safe_type_census_fails_closed_on_unresolved_cyclic_and_empty_corpora() {
    // A type the census does not recognise. Unresolved is a failure: a census
    // that skips what it cannot resolve has counted the easy fields.
    assert_eq!(
        census_governed_types(
            &corpus_with(&[(
                ERROR_ROOT,
                root_enum_with_extra_variant("Opaque", vec![("payload", "UnmodelledPayload")])
            )]),
            CENSUS_ROOTS
        ),
        Err(CensusReject::Unresolved {
            type_name: "UnmodelledPayload".to_string(),
            path: "PanelReceiptError::Opaque.payload".to_string(),
        })
    );

    // A cycle the census cannot traverse to a fixed point.
    assert_eq!(
        census_governed_types(
            &corpus_with(&[(
                "ProducerContext",
                TypeDef::Struct(vec![("stage", "PipelineStage"), ("parent", "RemedyPlan"),])
            )]),
            CENSUS_ROOTS
        ),
        Err(CensusReject::UnsupportedCycle {
            type_name: "RemedyPlan".to_string(),
            path: "RemedyPlan.producer.parent".to_string(),
        })
    );

    // An error enum with no variants: nothing was examined from that root, so
    // nothing was shown safe. The three other roots still contribute 8 variants
    // and 15 fields between them, so only a per-root rule rejects this.
    assert_eq!(
        census_governed_types(
            &corpus_with(&[(ERROR_ROOT, TypeDef::Enum(vec![]))]),
            CENSUS_ROOTS
        ),
        Err(CensusReject::RootExaminedNothing {
            root: ERROR_ROOT.to_string(),
        })
    );

    // An empty corpus.
    assert_eq!(
        census_governed_types(&TypeCorpus::new(), CENSUS_ROOTS),
        Err(CensusReject::EmptyCorpus)
    );

    // A declared root that is not in the corpus at all.
    assert_eq!(
        census_governed_types(&safe_panel_receipt_corpus(), &["AbsentGovernedType"]),
        Err(CensusReject::Unresolved {
            type_name: "AbsentGovernedType".to_string(),
            path: "AbsentGovernedType".to_string(),
        })
    );
}

#[test]
fn safe_type_census_fails_closed_on_a_root_set_that_governs_nothing() {
    // No declared roots. The root set is a list someone maintains, so an empty
    // one is a declaration bug: censusing nothing must never read as a pass.
    assert_eq!(
        census_governed_types(&safe_panel_receipt_corpus(), &[]),
        Err(CensusReject::NoRoots)
    );

    // One unresolvable root among otherwise valid ones. A governed type whose
    // name no longer resolves - renamed, moved, or deleted - fails the whole
    // census rather than quietly shrinking the governed set.
    assert_eq!(
        census_governed_types(
            &safe_panel_receipt_corpus(),
            &[ERROR_ROOT, "AbsentGovernedType", "RemedyPlan"]
        ),
        Err(CensusReject::Unresolved {
            type_name: "AbsentGovernedType".to_string(),
            path: "AbsentGovernedType".to_string(),
        })
    );

    // A root that resolves to a leaf newtype. It has no variant and no field,
    // so walking it proves nothing about any payload; the rule is that a root
    // must expose governable structure of its own.
    assert_eq!(
        census_governed_types(&safe_panel_receipt_corpus(), &["AttemptCount"]),
        Err(CensusReject::RootExaminedNothing {
            root: "AttemptCount".to_string(),
        })
    );
}

// ---------------------------------------------------------------------------
// panel-migrate refusal model + remedy-output fixtures.
//
// `make panel-migrate` does not exist in this repository yet, and neither does
// the wrapper this models. **Nothing below is the wrapper**, nothing below
// invokes git, reads a repository, or touches a worktree, and no assertion here
// is evidence about a shipped command. What lands now is the decision model,
// the renderer contract its refusals have to satisfy, and the corpus that
// exercises both - so the instructions that must never ship are rejected by a
// check that exists before the thing that would emit them.
//
// Two instructions are in question.
//
// The first is `git rebase <pinned-sha>`. The migration is pinned to a commit,
// so naming that pin as the rebase target reads correct and moves the
// contributor's branch *backwards* onto a historical commit, dropping every
// protected commit merged since. The pin is a **precondition** - the migration
// must be reachable from the fetched `origin/v3` - and never a target. A
// migration that is not yet published is a typed refusal, not a detour through
// the pin to get the files.
//
// The second is a bulk `git add` over the predicted conflict paths. A rebase
// replays commits one at a time and stops at whichever subset conflicts at the
// commit it is replaying, so the predicted set is the union across the whole
// replay and is never the working set at any single stop. A contributor who
// pastes a bulk add stages paths that are not unmerged at this stop, including
// files the replay has not reached, and turns a conflict resolution into an
// unrelated content change that `git rebase --continue` then commits. The
// predicted paths are therefore printed as an advisory planning list, and the
// only argument `git add` may carry is the literal placeholder
// `<resolved-paths-for-this-stop>`.
//
// So the audit below is deliberately two-sided. Positive: a conflict refusal
// must print the exact sorted paths it already computed, then `git fetch
// origin`, `git rebase origin/v3`, the per-stop `git status --short` /
// `git add <resolved-paths-for-this-stop>` / `git rebase --continue` sequence,
// the `git rebase --abort` way out, and the `make panel-migrate` rerun, in an
// order that works when run. Negative: every command line is **parsed** rather
// than keyword-scanned. An unrecognised subcommand or flag is a rejection, not
// a token to skip; a 40-hex object name anywhere on the line is a rejection,
// including inside `--onto=<sha>`, which is the backwards rebase wearing a
// flag; and a refusal that mutates nothing and offers no way forward yet may
// not carry a git command at all.
//
// **Wiring contract for the implementation commit.** The commit that writes the
// wrapper renders its refusals through a real renderer and feeds that output to
// `audit_refusal_output` - the same predicate, not a second one - keeping the
// planted outputs below as its negative corpus.
// ---------------------------------------------------------------------------

const PANEL_MIGRATE_COMMAND: &str = "make panel-migrate";
/// The only ref a refusal may name as a rebase target.
const PANEL_MIGRATE_TARGET_REF: &str = "origin/v3";
/// The only remote a refusal may name.
const PANEL_MIGRATE_REMOTE: &str = "origin";
/// `git fetch origin`, not `git fetch origin v3`: the refspec form updates
/// `FETCH_HEAD` without reliably updating `refs/remotes/origin/v3` on every
/// supported Git configuration, and the next line resolves `origin/v3`.
const PANEL_MIGRATE_FETCH: &str = "git fetch origin";
const PANEL_MIGRATE_REBASE: &str = "git rebase origin/v3";
const PANEL_MIGRATE_CONTINUE: &str = "git rebase --continue";
const PANEL_MIGRATE_ABORT: &str = "git rebase --abort";
const PANEL_MIGRATE_STATUS: &str = "git status --short";
const PANEL_MIGRATE_STASH_PUSH: &str = "git stash push -u -m panel-migrate";
const PANEL_MIGRATE_STASH_POP: &str = "git stash pop";
/// The only argument `git add` may carry. It is literal safe guidance, not a
/// path: the contributor substitutes whatever is unmerged at the stop they are
/// standing at, which is never known to be the predicted set.
const PANEL_MIGRATE_ADD_PLACEHOLDER: &str = "<resolved-paths-for-this-stop>";
const PANEL_MIGRATE_ADD: &str = "git add <resolved-paths-for-this-stop>";
/// The only git subcommands a refusal may name. Unrecognised is a rejection.
const PANEL_MIGRATE_SUBCOMMANDS: &[&str] = &["fetch", "rebase", "status", "add", "stash"];
const OBJECT_NAME_LEN: usize = 40;

/// A 40-hex object name. Modelled as its own type because the entire point of
/// this fixture is that an object name is a precondition and never a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ObjectName(&'static str);

/// `origin/v3` as the checkout had it before this run fetched.
const CURRENT_ORIGIN_V3: ObjectName = ObjectName("1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a");
/// `origin/v3` as this run's fetch resolved it: the only supported target.
const FETCHED_ORIGIN_V3: ObjectName = ObjectName("2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b");
/// The pinned panel migration. A precondition on the fetched target, and the
/// exact value a plausible-looking `git rebase <sha>` would name.
const PINNED_MIGRATION: ObjectName = ObjectName("3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c");

/// The planted would-conflict set, deliberately unsorted and bounded: the
/// refusal has to print what it computed, in a stable order, not echo git's.
const PLANTED_CONFLICT_PATHS: &[&str] = &[
    "skills/panel/adapter.mjs",
    "docs/contributing/copilot-agents.md",
    "skills/panel/SKILL.md",
];

/// What the wrapper found before deciding. `Conflicting` is the state this
/// fixture exists for; `Clean` is the only one that proceeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeState {
    Clean,
    Dirty,
    Conflicting,
}

/// The facts a decision is made from. Everything here is supplied by the
/// caller: nothing is read from a repository.
#[derive(Debug, Clone)]
struct MigrateContext {
    current_origin_v3: ObjectName,
    /// `origin/v3` as this run's fetch resolved it. `None` models a fetch that
    /// produced no such ref: no target, so no rebase.
    fetched_origin_v3: Option<ObjectName>,
    /// The pinned migration that must be present. Never a target.
    required_migration: ObjectName,
    /// Whether `required_migration` is reachable from `fetched_origin_v3`.
    migration_reachable: bool,
    state: TreeState,
    /// The paths the wrapper predicts would conflict, in the order it found
    /// them. Advisory: the union across the replay, never one stop's set.
    would_conflict: Vec<&'static str>,
}

#[derive(Debug, PartialEq, Eq)]
enum MigrateOutcome {
    /// The branch moves onto the fetched target ref, and onto nothing else.
    Rebase {
        target_ref: &'static str,
        from: ObjectName,
        onto: ObjectName,
    },
    Refuse(MigrateRefusal),
}

#[derive(Debug, PartialEq, Eq)]
enum MigrateRefusal {
    /// The fetch produced no `origin/v3`. There is no supported place to land,
    /// and the pin is not a substitute for one.
    TargetUnavailable {
        current: ObjectName,
    },
    /// The required migration is not reachable from the fetched target: it has
    /// not been published. Detaching to the pin to obtain the files would move
    /// the branch backwards, so this refusal offers no git command at all.
    UnpublishedMigration {
        required: ObjectName,
        fetched: ObjectName,
    },
    DirtyTree,
    /// The refusal that prints work already done: the predicted would-conflict
    /// paths, sorted, plus the per-stop sequence that resolves them.
    ConflictingUpdate {
        onto: ObjectName,
        paths: Vec<&'static str>,
    },
}

/// A caller that contradicts itself. Not a refusal - a refusal is a decision
/// about a repository, and these are decisions that were never made.
#[derive(Debug, PartialEq, Eq)]
enum MigrateFault {
    /// A conflicting state with no computed path. The refusal's whole value is
    /// printing the paths, so an empty list would render a refusal telling the
    /// contributor to go rediscover a conflict that has not happened yet.
    ConflictWithoutPaths,
    /// Paths computed for a state that is not conflicting.
    PathsWithoutConflictState { state: TreeState },
}

/// The decision, as a pure function of the facts.
///
/// Order matters: with no fetched target there is nothing to compare against,
/// and with the migration unpublished there is nothing supported to land on, so
/// both precede any statement about the working tree.
fn plan_migration(ctx: &MigrateContext) -> Result<MigrateOutcome, MigrateFault> {
    match ctx.state {
        TreeState::Conflicting if ctx.would_conflict.is_empty() => {
            return Err(MigrateFault::ConflictWithoutPaths);
        }
        TreeState::Clean | TreeState::Dirty if !ctx.would_conflict.is_empty() => {
            return Err(MigrateFault::PathsWithoutConflictState { state: ctx.state });
        }
        _ => {}
    }

    let Some(fetched) = ctx.fetched_origin_v3 else {
        return Ok(MigrateOutcome::Refuse(MigrateRefusal::TargetUnavailable {
            current: ctx.current_origin_v3,
        }));
    };
    if !ctx.migration_reachable {
        return Ok(MigrateOutcome::Refuse(
            MigrateRefusal::UnpublishedMigration {
                required: ctx.required_migration,
                fetched,
            },
        ));
    }

    Ok(match ctx.state {
        TreeState::Dirty => MigrateOutcome::Refuse(MigrateRefusal::DirtyTree),
        TreeState::Conflicting => {
            let mut paths = ctx.would_conflict.clone();
            paths.sort_unstable();
            paths.dedup();
            MigrateOutcome::Refuse(MigrateRefusal::ConflictingUpdate {
                onto: fetched,
                paths,
            })
        }
        TreeState::Clean => MigrateOutcome::Rebase {
            target_ref: PANEL_MIGRATE_TARGET_REF,
            from: ctx.current_origin_v3,
            onto: fetched,
        },
    })
}

/// Render one refusal. Object names appear only in prose diagnosis, never in a
/// command; the rebase target is spelled `origin/v3` at every site; and the
/// predicted paths are printed as a list and never pasted into a command.
fn render_refusal(refusal: &MigrateRefusal) -> Vec<String> {
    match refusal {
        MigrateRefusal::TargetUnavailable { current } => vec![
            "panel-migrate: refusing to migrate; nothing has been changed.".to_string(),
            format!(
                "Fetching {PANEL_MIGRATE_TARGET_REF} produced no such ref, so there is no \
                 supported branch to move onto."
            ),
            format!(
                "This checkout still has {PANEL_MIGRATE_TARGET_REF} at {}.",
                current.0
            ),
            "Restore access to the remote, then rerun:".to_string(),
            format!("  {PANEL_MIGRATE_COMMAND}"),
        ],
        MigrateRefusal::UnpublishedMigration { required, fetched } => vec![
            "panel-migrate: refusing to migrate; nothing has been changed.".to_string(),
            format!(
                "The required panel migration {} is not reachable from the fetched \
                 {PANEL_MIGRATE_TARGET_REF} {}.",
                required.0, fetched.0
            ),
            "It has not been published yet. That revision names the migration that must be \
             present, not a place to land: moving this branch onto it would drop every \
             protected commit merged since."
                .to_string(),
            format!("Wait for the migration to reach {PANEL_MIGRATE_TARGET_REF}, then rerun:"),
            format!("  {PANEL_MIGRATE_COMMAND}"),
        ],
        MigrateRefusal::DirtyTree => vec![
            "panel-migrate: refusing to migrate; the working tree has uncommitted changes."
                .to_string(),
            "Nothing has been changed. Review them:".to_string(),
            format!("  {PANEL_MIGRATE_STATUS}"),
            "Then either commit them, or set them aside:".to_string(),
            format!("  {PANEL_MIGRATE_STASH_PUSH}"),
            "Then rerun:".to_string(),
            format!("  {PANEL_MIGRATE_COMMAND}"),
            "If you stashed, restore them once it succeeds:".to_string(),
            format!("  {PANEL_MIGRATE_STASH_POP}"),
        ],
        MigrateRefusal::ConflictingUpdate { paths, .. } => {
            let mut out = vec![
                "panel-migrate: refusing to migrate; nothing has been changed.".to_string(),
                format!(
                    "Updating onto {PANEL_MIGRATE_TARGET_REF} is predicted to conflict in these \
                     paths:"
                ),
            ];
            out.extend(paths.iter().map(|path| format!("  {path}")));
            out.push(
                "That list is advisory. A rebase replays one commit at a time and stops at \
                 whichever of those paths conflict at that commit, never at all of them at once."
                    .to_string(),
            );
            out.push(
                "Move the branch forward yourself, starting from the protected branch:".to_string(),
            );
            out.push(format!("  {PANEL_MIGRATE_FETCH}"));
            out.push(format!("  {PANEL_MIGRATE_REBASE}"));
            out.push(
                "At each stop, review what is unmerged, resolve only those files, and continue:"
                    .to_string(),
            );
            out.push(format!("  {PANEL_MIGRATE_STATUS}"));
            out.push(format!("  {PANEL_MIGRATE_ADD}"));
            out.push(format!("  {PANEL_MIGRATE_CONTINUE}"));
            out.push("To abandon the migration at any stop:".to_string());
            out.push(format!("  {PANEL_MIGRATE_ABORT}"));
            out.push("Once the rebase finishes, rerun:".to_string());
            out.push(format!("  {PANEL_MIGRATE_COMMAND}"));
            out
        }
    }
}

/// Why a rendered refusal is not acceptable output.
#[derive(Debug, PartialEq, Eq)]
enum RefusalReject {
    /// The backwards-migration instruction: a rebase onto a 40-hex object name.
    /// It looks like a correct command and moves the branch behind protected
    /// `v3`, which is exactly what a human reading the output waves through.
    RebaseOntoObjectName { target: String },
    /// A rebase onto some ref that is not `origin/v3`.
    RebaseOntoForeignRef { target: String },
    /// `git rebase` with nothing at all after it.
    RebaseWithoutTarget,
    /// An object name inside some other git command - a `checkout`, a `reset` -
    /// which reaches the same backwards state by another route.
    ObjectNameInCommand { command: String },
    /// A git subcommand that is not on the allowed list. Unrecognised fails
    /// closed: an audit that skips what it cannot classify is an audit an
    /// unfamiliar instruction walks straight through.
    UnknownSubcommand { subcommand: String },
    /// A flag that is not on the allowed list for its subcommand.
    UnknownFlag { subcommand: String, flag: String },
    /// An allowed subcommand in a form the refusal may not print.
    UnexpectedCommandForm { command: String },
    /// `git fetch origin v3`: the refspec form updates `FETCH_HEAD` without
    /// reliably updating `refs/remotes/origin/v3`, which the next line resolves.
    FetchWithExplicitRefspec { refspec: String },
    /// A fetch from some remote other than `origin`.
    FetchFromForeignRemote { remote: String },
    /// A `git add` naming a predicted path. The predicted set is the union
    /// across the replay, so a literal path stages something that may not be
    /// unmerged at this stop.
    AddNamesPredictedPath { path: String },
    /// The bulk shape: one `git add` over the whole predicted set, which stages
    /// files the replay has not reached and turns a resolution into an
    /// unrelated change that `git rebase --continue` then commits.
    BulkAddOverPredictedPaths { paths: Vec<String> },
    /// A `git add` argument that is not the per-stop placeholder.
    AddArgumentNotPlaceholder { argument: String },
    /// A line the audit cannot place: it names a program without being a
    /// command line the parser accepts.
    UnclassifiedLine { line: String },
    /// A refusal with no supported way forward yet printed a git command.
    GitCommandInBlockedRefusal { command: String },
    /// A refusal that is not about conflicts told the contributor to rebase.
    UnexpectedRebaseInstruction { command: String },
    /// The output omits a command the contributor needs.
    MissingCommand { command: String },
    /// Two commands are printed in an order that does not work when run.
    CommandsOutOfOrder { earlier: String, later: String },
    /// The conflict refusal printed no path, sending the contributor to
    /// rediscover a conflict that has not happened yet.
    NoConflictPathsPrinted,
    /// A computed path is missing from the printed list.
    ConflictPathMissing { path: String },
    /// A printed path the wrapper never computed.
    UnexpectedConflictPath { path: String },
    /// The printed paths are in some order other than sorted.
    ConflictPathsUnsorted,
    /// The diagnosis does not name what it is about.
    MissingDiagnosis { needle: String },
}

/// A 40-hex object name, the shape that must never be a rebase target.
fn is_object_name(token: &str) -> bool {
    token.len() == OBJECT_NAME_LEN && token.chars().all(|c| c.is_ascii_hexdigit())
}

/// The first 40-hex object name anywhere on a command line: a positional
/// argument, a `--flag=<sha>` assignment, or a separated flag value. Scanning
/// the whole line rather than its positional arguments is the point - a
/// token-skipping scan passes `--onto=<sha>`, which is the backwards rebase
/// wearing a flag.
fn object_name_on_line(line: &str) -> Option<String> {
    let mut run = String::new();
    for character in line.chars().chain(std::iter::once(' ')) {
        if character.is_ascii_hexdigit() {
            run.push(character);
            continue;
        }
        if run.len() >= OBJECT_NAME_LEN {
            return Some(run);
        }
        run.clear();
    }
    None
}

/// The steps a refusal is allowed to name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitStep {
    Fetch,
    Rebase,
    Status,
    Add,
    Continue,
    Abort,
    StashPush,
    StashPop,
    Rerun,
}

/// The one spelling of each step. A rendered command line either is this
/// string, or it is not that step.
fn step_command(step: GitStep) -> &'static str {
    match step {
        GitStep::Fetch => PANEL_MIGRATE_FETCH,
        GitStep::Rebase => PANEL_MIGRATE_REBASE,
        GitStep::Status => PANEL_MIGRATE_STATUS,
        GitStep::Add => PANEL_MIGRATE_ADD,
        GitStep::Continue => PANEL_MIGRATE_CONTINUE,
        GitStep::Abort => PANEL_MIGRATE_ABORT,
        GitStep::StashPush => PANEL_MIGRATE_STASH_PUSH,
        GitStep::StashPop => PANEL_MIGRATE_STASH_POP,
        GitStep::Rerun => PANEL_MIGRATE_COMMAND,
    }
}

/// The exact flags each allowed subcommand may carry. Everything else fails
/// closed, including `--onto`, which is how a backwards rebase gets printed
/// without a bare revision in an argument position.
fn allowed_flags(subcommand: &str) -> &'static [&'static str] {
    match subcommand {
        "rebase" => &["--continue", "--abort"],
        "status" => &["--short"],
        "stash" => &["-u", "-m"],
        _ => &[],
    }
}

/// The command a line gives an operator, if it gives one.
fn migrate_command(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    (trimmed.starts_with("git ") || trimmed.starts_with("make ")).then_some(trimmed)
}

/// Parse one command line into the step it names, or say exactly why it is not
/// a step a refusal may print. Every subcommand, flag, ref, and argument is
/// checked against a closed list.
fn parse_command(command: &str, predicted: &[&str]) -> Result<GitStep, RefusalReject> {
    if let Some(name) = object_name_on_line(command) {
        return Err(if command.starts_with("git rebase") {
            RefusalReject::RebaseOntoObjectName { target: name }
        } else {
            RefusalReject::ObjectNameInCommand {
                command: command.to_string(),
            }
        });
    }

    let unexpected = || RefusalReject::UnexpectedCommandForm {
        command: command.to_string(),
    };
    let mut tokens = command.split_whitespace();
    let program = tokens.next().unwrap_or_default();
    let args: Vec<&str> = tokens.collect();

    if program == "make" {
        return (command == PANEL_MIGRATE_COMMAND)
            .then_some(GitStep::Rerun)
            .ok_or_else(unexpected);
    }

    let Some((subcommand, rest)) = args.split_first() else {
        return Err(unexpected());
    };
    if !PANEL_MIGRATE_SUBCOMMANDS.contains(subcommand) {
        return Err(RefusalReject::UnknownSubcommand {
            subcommand: (*subcommand).to_string(),
        });
    }
    if let Some(flag) = rest
        .iter()
        .filter(|arg| arg.starts_with('-'))
        .find(|arg| !allowed_flags(subcommand).contains(arg))
    {
        return Err(RefusalReject::UnknownFlag {
            subcommand: (*subcommand).to_string(),
            flag: (*flag).to_string(),
        });
    }

    match *subcommand {
        "fetch" => {
            let Some(remote) = rest.first() else {
                return Err(unexpected());
            };
            if *remote != PANEL_MIGRATE_REMOTE {
                return Err(RefusalReject::FetchFromForeignRemote {
                    remote: (*remote).to_string(),
                });
            }
            if let Some(refspec) = rest.get(1) {
                return Err(RefusalReject::FetchWithExplicitRefspec {
                    refspec: (*refspec).to_string(),
                });
            }
            Ok(GitStep::Fetch)
        }
        "rebase" => {
            if rest.is_empty() {
                return Err(RefusalReject::RebaseWithoutTarget);
            }
            let (flags, targets): (Vec<&str>, Vec<&str>) =
                rest.iter().copied().partition(|arg| arg.starts_with('-'));
            if !flags.is_empty() {
                // `--continue` and `--abort` name no target and take none.
                if flags.len() != 1 || !targets.is_empty() {
                    return Err(unexpected());
                }
                return match flags[0] {
                    "--continue" => Ok(GitStep::Continue),
                    "--abort" => Ok(GitStep::Abort),
                    flag => Err(RefusalReject::UnknownFlag {
                        subcommand: (*subcommand).to_string(),
                        flag: flag.to_string(),
                    }),
                };
            }
            if targets.len() != 1 {
                return Err(unexpected());
            }
            if targets[0] != PANEL_MIGRATE_TARGET_REF {
                return Err(RefusalReject::RebaseOntoForeignRef {
                    target: targets[0].to_string(),
                });
            }
            Ok(GitStep::Rebase)
        }
        "status" => (command == PANEL_MIGRATE_STATUS)
            .then_some(GitStep::Status)
            .ok_or_else(unexpected),
        "add" => {
            if rest.is_empty() {
                return Err(unexpected());
            }
            let named: Vec<&str> = rest
                .iter()
                .copied()
                .filter(|arg| predicted.contains(arg))
                .collect();
            if !predicted.is_empty() && named.len() == rest.len() && named.len() == predicted.len()
            {
                return Err(RefusalReject::BulkAddOverPredictedPaths {
                    paths: named.iter().map(|path| (*path).to_string()).collect(),
                });
            }
            if let Some(path) = named.first() {
                return Err(RefusalReject::AddNamesPredictedPath {
                    path: (*path).to_string(),
                });
            }
            if let Some(argument) = rest
                .iter()
                .find(|arg| **arg != PANEL_MIGRATE_ADD_PLACEHOLDER)
            {
                return Err(RefusalReject::AddArgumentNotPlaceholder {
                    argument: (*argument).to_string(),
                });
            }
            if rest.len() != 1 {
                return Err(unexpected());
            }
            Ok(GitStep::Add)
        }
        "stash" => {
            if command == PANEL_MIGRATE_STASH_PUSH {
                Ok(GitStep::StashPush)
            } else if command == PANEL_MIGRATE_STASH_POP {
                Ok(GitStep::StashPop)
            } else {
                Err(unexpected())
            }
        }
        other => Err(RefusalReject::UnknownSubcommand {
            subcommand: other.to_string(),
        }),
    }
}

/// What one line of a rendered refusal is.
#[derive(Debug)]
enum RefusalLine<'a> {
    Diagnosis,
    Path(&'a str),
    Command(GitStep),
}

/// Place one line, or fail closed. A line that names `git` or `make` without
/// being a command line the parser accepts - `sudo git rebase …`, an
/// instruction buried in prose - is a rejection rather than something to skip.
fn classify_line<'a>(line: &'a str, predicted: &[&str]) -> Result<RefusalLine<'a>, RefusalReject> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(RefusalLine::Diagnosis);
    }
    if let Some(command) = migrate_command(trimmed) {
        return parse_command(command, predicted).map(RefusalLine::Command);
    }

    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens
        .iter()
        .any(|token| *token == "git" || *token == "make")
    {
        return Err(RefusalReject::UnclassifiedLine {
            line: trimmed.to_string(),
        });
    }
    Ok(if tokens.len() == 1 {
        RefusalLine::Path(trimmed)
    } else {
        RefusalLine::Diagnosis
    })
}

fn command_position(output: &[String], command: &str) -> Option<usize> {
    output
        .iter()
        .position(|line| migrate_command(line) == Some(command))
}

/// `steps` are `(step, line index)` in the order they must be run. An order
/// that reads fine and does not work when run is not a remedy.
fn ordered_steps(steps: &[(GitStep, usize)]) -> Result<(), RefusalReject> {
    for pair in steps.windows(2) {
        if pair[0].1 > pair[1].1 {
            return Err(RefusalReject::CommandsOutOfOrder {
                earlier: step_command(pair[0].0).to_string(),
                later: step_command(pair[1].0).to_string(),
            });
        }
    }
    Ok(())
}

/// What the audit examined. Reported rather than asserted away: an audit that
/// cannot say what it checked cannot be shown to have checked anything.
#[derive(Debug, PartialEq, Eq)]
struct RefusalAudit {
    lines: usize,
    commands: usize,
    paths: usize,
}

/// Accept a rendered refusal, or say exactly why it is not acceptable.
///
/// `refusal` is the decision the model made; `output` is the text some renderer
/// produced for it, which is why the planted-output fixtures below can hand
/// this a real refusal with text no correct renderer would emit.
fn audit_refusal_output(
    refusal: &MigrateRefusal,
    output: &[String],
) -> Result<RefusalAudit, RefusalReject> {
    let predicted: &[&str] = match refusal {
        MigrateRefusal::ConflictingUpdate { paths, .. } => paths,
        _ => &[],
    };

    // Every line is placed before anything is required of the output: the
    // parse is what rejects an object name, an unknown subcommand or flag, a
    // foreign ref, and a literal path pasted into `git add`.
    let mut printed: Vec<&str> = Vec::new();
    let mut steps: Vec<(GitStep, usize)> = Vec::new();
    for (index, line) in output.iter().enumerate() {
        match classify_line(line, predicted)? {
            RefusalLine::Diagnosis => {}
            RefusalLine::Path(path) => printed.push(path),
            RefusalLine::Command(step) => steps.push((step, index)),
        }
    }

    let require = |step: GitStep| -> Result<usize, RefusalReject> {
        steps
            .iter()
            .find(|(found, _)| *found == step)
            .map(|(_, index)| *index)
            .ok_or_else(|| RefusalReject::MissingCommand {
                command: step_command(step).to_string(),
            })
    };
    let mention = |needle: &str| -> Result<(), RefusalReject> {
        output
            .iter()
            .any(|line| line.contains(needle))
            .then_some(())
            .ok_or_else(|| RefusalReject::MissingDiagnosis {
                needle: needle.to_string(),
            })
    };
    let no_git_commands = || -> Result<(), RefusalReject> {
        match steps
            .iter()
            .map(|(step, _)| *step)
            .find(|step| step_command(*step).starts_with("git "))
        {
            Some(step) => Err(RefusalReject::GitCommandInBlockedRefusal {
                command: step_command(step).to_string(),
            }),
            None => Ok(()),
        }
    };
    let no_rebase = || -> Result<(), RefusalReject> {
        match steps
            .iter()
            .map(|(step, _)| *step)
            .find(|step| matches!(step, GitStep::Rebase | GitStep::Continue | GitStep::Abort))
        {
            Some(step) => Err(RefusalReject::UnexpectedRebaseInstruction {
                command: step_command(step).to_string(),
            }),
            None => Ok(()),
        }
    };

    match refusal {
        MigrateRefusal::TargetUnavailable { current } => {
            mention(PANEL_MIGRATE_TARGET_REF)?;
            mention(current.0)?;
            no_git_commands()?;
            require(GitStep::Rerun)?;
        }
        MigrateRefusal::UnpublishedMigration { required, fetched } => {
            mention(PANEL_MIGRATE_TARGET_REF)?;
            mention(required.0)?;
            mention(fetched.0)?;
            // The pin is named as the missing precondition and nowhere else:
            // no fetch, no rebase, no detour to obtain the files.
            no_git_commands()?;
            require(GitStep::Rerun)?;
        }
        MigrateRefusal::DirtyTree => {
            no_rebase()?;
            let status = require(GitStep::Status)?;
            let stash = require(GitStep::StashPush)?;
            let rerun = require(GitStep::Rerun)?;
            let pop = require(GitStep::StashPop)?;
            ordered_steps(&[
                (GitStep::Status, status),
                (GitStep::StashPush, stash),
                (GitStep::Rerun, rerun),
                (GitStep::StashPop, pop),
            ])?;
        }
        MigrateRefusal::ConflictingUpdate { paths, .. } => {
            if printed.is_empty() {
                return Err(RefusalReject::NoConflictPathsPrinted);
            }
            for path in paths {
                if !printed.iter().any(|printed| printed == path) {
                    return Err(RefusalReject::ConflictPathMissing {
                        path: (*path).to_string(),
                    });
                }
            }
            for printed_path in &printed {
                if !paths.iter().any(|path| path == printed_path) {
                    return Err(RefusalReject::UnexpectedConflictPath {
                        path: (*printed_path).to_string(),
                    });
                }
            }
            if !printed.is_sorted() {
                return Err(RefusalReject::ConflictPathsUnsorted);
            }

            let fetch = require(GitStep::Fetch)?;
            let rebase = require(GitStep::Rebase)?;
            let status = require(GitStep::Status)?;
            let add = require(GitStep::Add)?;
            let continued = require(GitStep::Continue)?;
            let abort = require(GitStep::Abort)?;
            let rerun = require(GitStep::Rerun)?;

            // The whole constraint set, not a membership check: fetch before
            // the rebase, the rebase before the per-stop loop, and the rerun
            // after whichever branch the contributor takes out of it.
            ordered_steps(&[
                (GitStep::Fetch, fetch),
                (GitStep::Rebase, rebase),
                (GitStep::Status, status),
                (GitStep::Add, add),
                (GitStep::Continue, continued),
                (GitStep::Rerun, rerun),
            ])?;
            ordered_steps(&[
                (GitStep::Rebase, rebase),
                (GitStep::Abort, abort),
                (GitStep::Rerun, rerun),
            ])?;
        }
    }

    Ok(RefusalAudit {
        lines: output.len(),
        commands: steps.len(),
        paths: printed.len(),
    })
}

// --- in-memory fixtures for the refusal corpus -----------------------------

/// A self-consistent context: fetched target present, migration published, and
/// would-conflict paths exactly when the state is `Conflicting`.
fn migrate_context(state: TreeState) -> MigrateContext {
    MigrateContext {
        current_origin_v3: CURRENT_ORIGIN_V3,
        fetched_origin_v3: Some(FETCHED_ORIGIN_V3),
        required_migration: PINNED_MIGRATION,
        migration_reachable: true,
        state,
        would_conflict: match state {
            TreeState::Conflicting => PLANTED_CONFLICT_PATHS.to_vec(),
            TreeState::Clean | TreeState::Dirty => Vec::new(),
        },
    }
}

/// The five states the model distinguishes: the one that proceeds, and the four
/// typed refusals. Labelled so the counted corpus reports what it examined.
fn migrate_corpus() -> Vec<(&'static str, MigrateContext)> {
    let mut unpublished = migrate_context(TreeState::Clean);
    unpublished.migration_reachable = false;
    let mut unavailable = migrate_context(TreeState::Clean);
    unavailable.fetched_origin_v3 = None;

    vec![
        ("clean tree", migrate_context(TreeState::Clean)),
        ("dirty tree", migrate_context(TreeState::Dirty)),
        (
            "conflicting update",
            migrate_context(TreeState::Conflicting),
        ),
        ("unpublished migration", unpublished),
        ("target unavailable", unavailable),
    ]
}

fn refusal_for(ctx: &MigrateContext) -> MigrateRefusal {
    match plan_migration(ctx).expect("the fixture context is self-consistent") {
        MigrateOutcome::Refuse(refusal) => refusal,
        outcome => panic!("expected a refusal, got {outcome:?}"),
    }
}

fn conflict_refusal() -> MigrateRefusal {
    refusal_for(&migrate_context(TreeState::Conflicting))
}

fn unavailable_refusal() -> MigrateRefusal {
    let mut ctx = migrate_context(TreeState::Clean);
    ctx.fetched_origin_v3 = None;
    refusal_for(&ctx)
}

/// Drop every line whose command starts with `prefix`.
fn without_command(output: &[String], prefix: &str) -> Vec<String> {
    output
        .iter()
        .filter(|line| !line.trim().starts_with(prefix))
        .cloned()
        .collect()
}

/// Drop every line that is exactly `line` once trimmed.
fn without_line(output: &[String], dropped: &str) -> Vec<String> {
    output
        .iter()
        .filter(|line| line.trim() != dropped)
        .cloned()
        .collect()
}

/// Replace the line that is exactly `target` with `replacement`, keeping the
/// indent the renderer uses. This is how a planted renderer output is built:
/// one line of otherwise-correct output says something it must not say.
fn planting(output: &[String], target: &str, replacement: &str) -> Vec<String> {
    let planted: Vec<String> = output
        .iter()
        .map(|line| {
            if line.trim() == target {
                format!("  {replacement}")
            } else {
                line.clone()
            }
        })
        .collect();
    assert_ne!(
        planted.as_slice(),
        output,
        "the planted fixture must actually change the output; `{target}` was not there"
    );
    planted
}

/// Swap the two lines carrying these commands, leaving every other line where
/// it is.
fn swapping(output: &[String], first: &str, second: &str) -> Vec<String> {
    let earlier = command_position(output, first).expect("the first command is in the output");
    let later = command_position(output, second).expect("the second command is in the output");
    let mut planted = output.to_vec();
    planted.swap(earlier, later);
    planted
}

/// Move the line carrying `command` to sit immediately before `anchor`.
fn moving_before(output: &[String], command: &str, anchor: &str) -> Vec<String> {
    let from = command_position(output, command).expect("the moved command is in the output");
    let mut planted = output.to_vec();
    let line = planted.remove(from);
    let to = command_position(&planted, anchor).expect("the anchor command is in the output");
    planted.insert(to, line);
    planted
}

/// Move the line carrying `command` to sit immediately after `anchor`.
fn moving_after(output: &[String], command: &str, anchor: &str) -> Vec<String> {
    let from = command_position(output, command).expect("the moved command is in the output");
    let mut planted = output.to_vec();
    let line = planted.remove(from);
    let to = command_position(&planted, anchor).expect("the anchor command is in the output");
    planted.insert(to + 1, line);
    planted
}

#[test]
fn panel_migrate_conflict_refusal_prints_the_paths_it_computed() {
    let refusal = conflict_refusal();

    // The computed set is sorted, whatever order it was found in.
    assert_eq!(
        refusal,
        MigrateRefusal::ConflictingUpdate {
            onto: FETCHED_ORIGIN_V3,
            paths: vec![
                "docs/contributing/copilot-agents.md",
                "skills/panel/SKILL.md",
                "skills/panel/adapter.mjs",
            ],
        },
        "the refusal names the fetched target, and its paths are sorted rather than in the \
         order they were computed"
    );

    // Exact content and exact order. The predicted paths are printed because
    // the wrapper already computed them: sending the contributor to run
    // `git status` on an untouched tree asks them to rediscover work already
    // done, and shows nothing, because the conflict has not happened yet. The
    // `git status --short` below is the *per-stop* review inside a rebase that
    // has actually stopped, which is a different thing at a different time.
    let rendered = render_refusal(&refusal);
    assert_eq!(
        rendered,
        vec![
            "panel-migrate: refusing to migrate; nothing has been changed.",
            "Updating onto origin/v3 is predicted to conflict in these paths:",
            "  docs/contributing/copilot-agents.md",
            "  skills/panel/SKILL.md",
            "  skills/panel/adapter.mjs",
            "That list is advisory. A rebase replays one commit at a time and stops at whichever \
             of those paths conflict at that commit, never at all of them at once.",
            "Move the branch forward yourself, starting from the protected branch:",
            "  git fetch origin",
            "  git rebase origin/v3",
            "At each stop, review what is unmerged, resolve only those files, and continue:",
            "  git status --short",
            "  git add <resolved-paths-for-this-stop>",
            "  git rebase --continue",
            "To abandon the migration at any stop:",
            "  git rebase --abort",
            "Once the rebase finishes, rerun:",
            "  make panel-migrate",
        ]
    );

    // The fetch is the plain form, and the add carries the placeholder rather
    // than any predicted path.
    let add = rendered
        .iter()
        .find(|line| line.trim().starts_with("git add"))
        .expect("the per-stop sequence names an add");
    assert_eq!(add.trim(), PANEL_MIGRATE_ADD);
    assert!(
        PLANTED_CONFLICT_PATHS
            .iter()
            .all(|path| !add.contains(path)),
        "the predicted set is the union across the replay, so no predicted path may be pasted \
         into a command the contributor runs at one stop: {add}"
    );
    assert!(
        !rendered
            .iter()
            .any(|line| line.trim() == "git fetch origin v3"),
        "the refspec form updates FETCH_HEAD without reliably updating refs/remotes/origin/v3"
    );

    assert_eq!(
        audit_refusal_output(&refusal, &rendered),
        Ok(RefusalAudit {
            lines: 17,
            commands: 7,
            paths: 3,
        })
    );
}

#[test]
fn panel_migrate_rejects_a_rebase_onto_anything_but_the_fetched_branch() {
    let refusal = conflict_refusal();
    let accepted = render_refusal(&refusal);

    // The exact backwards-migration instruction: the pinned migration commit,
    // which is a precondition on `origin/v3` and never a place to land.
    assert_eq!(
        audit_refusal_output(
            &refusal,
            &planting(
                &accepted,
                PANEL_MIGRATE_REBASE,
                &format!("git rebase {}", PINNED_MIGRATION.0)
            )
        ),
        Err(RefusalReject::RebaseOntoObjectName {
            target: PINNED_MIGRATION.0.to_string(),
        })
    );

    // The same instruction wearing a flag - assigned, separated, or on some
    // other flag entirely - and with an unrelated object name. A scan that only
    // inspects positional arguments passes the first of these.
    for planted in [
        format!("git rebase --onto={} origin/v3", PINNED_MIGRATION.0),
        format!("git rebase --onto {} origin/v3", PINNED_MIGRATION.0),
        format!("git rebase --hard={}", PINNED_MIGRATION.0),
        format!("git rebase {}", CURRENT_ORIGIN_V3.0),
    ] {
        assert!(
            matches!(
                audit_refusal_output(
                    &refusal,
                    &planting(&accepted, PANEL_MIGRATE_REBASE, &planted)
                ),
                Err(RefusalReject::RebaseOntoObjectName { .. })
            ),
            "a 40-hex rebase target is rejected however it is spelled: {planted}"
        );
    }

    // Any other ref - foreign remote or foreign branch - and a rebase with no
    // target at all.
    for foreign in ["upstream/main", "origin/main"] {
        assert_eq!(
            audit_refusal_output(
                &refusal,
                &planting(
                    &accepted,
                    PANEL_MIGRATE_REBASE,
                    &format!("git rebase {foreign}")
                )
            ),
            Err(RefusalReject::RebaseOntoForeignRef {
                target: foreign.to_string(),
            })
        );
    }
    assert_eq!(
        audit_refusal_output(
            &refusal,
            &planting(&accepted, PANEL_MIGRATE_REBASE, "git rebase")
        ),
        Err(RefusalReject::RebaseWithoutTarget)
    );

    // The same backwards move by another route, positionally and in a flag
    // assignment.
    for planted in [
        format!("git checkout {}", PINNED_MIGRATION.0),
        format!("git reset --hard={}", PINNED_MIGRATION.0),
    ] {
        assert_eq!(
            audit_refusal_output(
                &refusal,
                &planting(&accepted, PANEL_MIGRATE_REBASE, &planted)
            ),
            Err(RefusalReject::ObjectNameInCommand {
                command: planted.clone(),
            })
        );
    }

    // The accepted output names `origin/v3` as its only rebase target, so none
    // of the above is rejected for some incidental reason.
    assert!(audit_refusal_output(&refusal, &accepted).is_ok());
}

#[test]
fn panel_migrate_rejects_unrecognised_commands_flags_and_forms() {
    let refusal = conflict_refusal();
    let accepted = render_refusal(&refusal);

    let cases: Vec<(&str, Vec<String>, RefusalReject)> = vec![
        (
            "the superseded fetch form, which leaves refs/remotes/origin/v3 alone",
            planting(&accepted, PANEL_MIGRATE_FETCH, "git fetch origin v3"),
            RefusalReject::FetchWithExplicitRefspec {
                refspec: "v3".to_string(),
            },
        ),
        (
            "a fetch from a foreign remote",
            planting(&accepted, PANEL_MIGRATE_FETCH, "git fetch upstream"),
            RefusalReject::FetchFromForeignRemote {
                remote: "upstream".to_string(),
            },
        ),
        (
            "a fetch carrying a flag nobody allowed",
            planting(&accepted, PANEL_MIGRATE_FETCH, "git fetch --all origin"),
            RefusalReject::UnknownFlag {
                subcommand: "fetch".to_string(),
                flag: "--all".to_string(),
            },
        ),
        (
            "a subcommand that is not on the list",
            planting(&accepted, PANEL_MIGRATE_REBASE, "git cherry-pick origin/v3"),
            RefusalReject::UnknownSubcommand {
                subcommand: "cherry-pick".to_string(),
            },
        ),
        (
            "a rebase flag that is not on the list",
            planting(
                &accepted,
                PANEL_MIGRATE_REBASE,
                "git rebase --autosquash origin/v3",
            ),
            RefusalReject::UnknownFlag {
                subcommand: "rebase".to_string(),
                flag: "--autosquash".to_string(),
            },
        ),
        (
            "a status that is not the short form",
            planting(&accepted, PANEL_MIGRATE_STATUS, "git status"),
            RefusalReject::UnexpectedCommandForm {
                command: "git status".to_string(),
            },
        ),
        (
            "a status flag that is not the short form",
            planting(&accepted, PANEL_MIGRATE_STATUS, "git status --porcelain"),
            RefusalReject::UnknownFlag {
                subcommand: "status".to_string(),
                flag: "--porcelain".to_string(),
            },
        ),
        (
            "the rerun replaced by some other make target",
            planting(&accepted, PANEL_MIGRATE_COMMAND, "make check"),
            RefusalReject::UnexpectedCommandForm {
                command: "make check".to_string(),
            },
        ),
        (
            "a command the parser cannot place",
            planting(&accepted, PANEL_MIGRATE_REBASE, "sudo git rebase origin/v3"),
            RefusalReject::UnclassifiedLine {
                line: "sudo git rebase origin/v3".to_string(),
            },
        ),
    ];

    for (label, planted, expected) in cases {
        assert_eq!(
            audit_refusal_output(&refusal, &planted),
            Err(expected),
            "an unrecognised command line must fail closed rather than be skipped ({label})"
        );
    }
}

#[test]
fn panel_migrate_rejects_a_bulk_add_over_the_predicted_paths() {
    let refusal = conflict_refusal();
    let accepted = render_refusal(&refusal);
    let sorted: Vec<&str> = {
        let mut paths = PLANTED_CONFLICT_PATHS.to_vec();
        paths.sort_unstable();
        paths
    };

    // The bulk shape: one add over the whole predicted set. It stages paths
    // that are not unmerged at this stop, including files the replay has not
    // reached, which `git rebase --continue` then commits.
    assert_eq!(
        audit_refusal_output(
            &refusal,
            &planting(
                &accepted,
                PANEL_MIGRATE_ADD,
                &format!("git add {}", sorted.join(" "))
            )
        ),
        Err(RefusalReject::BulkAddOverPredictedPaths {
            paths: sorted.iter().map(|path| (*path).to_string()).collect(),
        })
    );

    // One predicted path is no better: it is still a prediction about some
    // other stop.
    assert_eq!(
        audit_refusal_output(
            &refusal,
            &planting(
                &accepted,
                PANEL_MIGRATE_ADD,
                "git add skills/panel/SKILL.md"
            )
        ),
        Err(RefusalReject::AddNamesPredictedPath {
            path: "skills/panel/SKILL.md".to_string(),
        })
    );

    // And any other literal argument, including the everything shapes.
    for (planted, argument) in [
        ("git add packages/Cargo.toml", "packages/Cargo.toml"),
        ("git add .", "."),
    ] {
        assert_eq!(
            audit_refusal_output(&refusal, &planting(&accepted, PANEL_MIGRATE_ADD, planted)),
            Err(RefusalReject::AddArgumentNotPlaceholder {
                argument: argument.to_string(),
            })
        );
    }
    assert_eq!(
        audit_refusal_output(
            &refusal,
            &planting(&accepted, PANEL_MIGRATE_ADD, "git add -A")
        ),
        Err(RefusalReject::UnknownFlag {
            subcommand: "add".to_string(),
            flag: "-A".to_string(),
        })
    );

    // The placeholder itself is the one accepted argument.
    assert!(audit_refusal_output(&refusal, &accepted).is_ok());
}

#[test]
fn panel_migrate_rejects_conflict_output_missing_any_remedy_step() {
    let refusal = conflict_refusal();
    let accepted = render_refusal(&refusal);

    let cases: Vec<(&str, Vec<String>, RefusalReject)> = vec![
        (
            "fetch dropped",
            without_command(&accepted, PANEL_MIGRATE_FETCH),
            RefusalReject::MissingCommand {
                command: PANEL_MIGRATE_FETCH.to_string(),
            },
        ),
        (
            "rebase onto origin/v3 dropped",
            without_command(&accepted, PANEL_MIGRATE_REBASE),
            RefusalReject::MissingCommand {
                command: PANEL_MIGRATE_REBASE.to_string(),
            },
        ),
        (
            "the per-stop review dropped",
            without_command(&accepted, PANEL_MIGRATE_STATUS),
            RefusalReject::MissingCommand {
                command: PANEL_MIGRATE_STATUS.to_string(),
            },
        ),
        (
            "add dropped",
            without_command(&accepted, PANEL_MIGRATE_ADD),
            RefusalReject::MissingCommand {
                command: PANEL_MIGRATE_ADD.to_string(),
            },
        ),
        (
            "continue dropped",
            without_command(&accepted, PANEL_MIGRATE_CONTINUE),
            RefusalReject::MissingCommand {
                command: PANEL_MIGRATE_CONTINUE.to_string(),
            },
        ),
        (
            "abort dropped",
            without_command(&accepted, PANEL_MIGRATE_ABORT),
            RefusalReject::MissingCommand {
                command: PANEL_MIGRATE_ABORT.to_string(),
            },
        ),
        (
            "rerun dropped",
            without_command(&accepted, PANEL_MIGRATE_COMMAND),
            RefusalReject::MissingCommand {
                command: PANEL_MIGRATE_COMMAND.to_string(),
            },
        ),
        (
            "one computed path never printed",
            without_line(&accepted, "skills/panel/SKILL.md"),
            RefusalReject::ConflictPathMissing {
                path: "skills/panel/SKILL.md".to_string(),
            },
        ),
        (
            "a path nobody computed printed",
            {
                let mut planted = accepted.clone();
                planted.insert(5, "  skills/panel/unrelated.mjs".to_string());
                planted
            },
            RefusalReject::UnexpectedConflictPath {
                path: "skills/panel/unrelated.mjs".to_string(),
            },
        ),
        (
            "paths printed in the order git happened to report them",
            {
                let mut planted = accepted.clone();
                planted.swap(2, 4);
                planted
            },
            RefusalReject::ConflictPathsUnsorted,
        ),
    ];

    for (label, planted, expected) in cases {
        assert_eq!(
            audit_refusal_output(&refusal, &planted),
            Err(expected),
            "planted conflict output was not rejected as expected ({label})"
        );
    }
}

#[test]
fn panel_migrate_rejects_every_out_of_order_conflict_rendering() {
    let refusal = conflict_refusal();
    let accepted = render_refusal(&refusal);

    let cases: Vec<(&str, Vec<String>, RefusalReject)> = vec![
        (
            "rebase before the fetch that resolves its target",
            swapping(&accepted, PANEL_MIGRATE_FETCH, PANEL_MIGRATE_REBASE),
            RefusalReject::CommandsOutOfOrder {
                earlier: PANEL_MIGRATE_FETCH.to_string(),
                later: PANEL_MIGRATE_REBASE.to_string(),
            },
        ),
        (
            "the per-stop review before the rebase that stops",
            moving_before(&accepted, PANEL_MIGRATE_STATUS, PANEL_MIGRATE_FETCH),
            RefusalReject::CommandsOutOfOrder {
                earlier: PANEL_MIGRATE_REBASE.to_string(),
                later: PANEL_MIGRATE_STATUS.to_string(),
            },
        ),
        (
            "add before the review that says what is unmerged",
            swapping(&accepted, PANEL_MIGRATE_STATUS, PANEL_MIGRATE_ADD),
            RefusalReject::CommandsOutOfOrder {
                earlier: PANEL_MIGRATE_STATUS.to_string(),
                later: PANEL_MIGRATE_ADD.to_string(),
            },
        ),
        (
            "continue before the add that resolves the stop",
            swapping(&accepted, PANEL_MIGRATE_ADD, PANEL_MIGRATE_CONTINUE),
            RefusalReject::CommandsOutOfOrder {
                earlier: PANEL_MIGRATE_ADD.to_string(),
                later: PANEL_MIGRATE_CONTINUE.to_string(),
            },
        ),
        (
            "continue before the rebase it continues",
            moving_before(&accepted, PANEL_MIGRATE_CONTINUE, PANEL_MIGRATE_REBASE),
            RefusalReject::CommandsOutOfOrder {
                earlier: PANEL_MIGRATE_ADD.to_string(),
                later: PANEL_MIGRATE_CONTINUE.to_string(),
            },
        ),
        (
            "the rerun before the rebase has finished",
            swapping(&accepted, PANEL_MIGRATE_CONTINUE, PANEL_MIGRATE_COMMAND),
            RefusalReject::CommandsOutOfOrder {
                earlier: PANEL_MIGRATE_CONTINUE.to_string(),
                later: PANEL_MIGRATE_COMMAND.to_string(),
            },
        ),
        (
            "abort before the rebase there is nothing yet to abandon",
            moving_before(&accepted, PANEL_MIGRATE_ABORT, PANEL_MIGRATE_REBASE),
            RefusalReject::CommandsOutOfOrder {
                earlier: PANEL_MIGRATE_REBASE.to_string(),
                later: PANEL_MIGRATE_ABORT.to_string(),
            },
        ),
        (
            "the rerun before the way out of the rebase",
            moving_after(&accepted, PANEL_MIGRATE_ABORT, PANEL_MIGRATE_COMMAND),
            RefusalReject::CommandsOutOfOrder {
                earlier: PANEL_MIGRATE_ABORT.to_string(),
                later: PANEL_MIGRATE_COMMAND.to_string(),
            },
        ),
    ];

    for (label, planted, expected) in cases {
        assert_eq!(
            audit_refusal_output(&refusal, &planted),
            Err(expected),
            "an order that reads fine and does not work when run is not a remedy ({label})"
        );
    }

    // Continue and abort are alternative branches out of the same stop, so
    // their order relative to each other is not a constraint - both being after
    // the rebase is.
    assert!(
        audit_refusal_output(
            &refusal,
            &swapping(&accepted, PANEL_MIGRATE_CONTINUE, PANEL_MIGRATE_ABORT)
        )
        .is_ok(),
        "the ordering constraint set is exactly the set that matters when run"
    );
}

#[test]
fn panel_migrate_conflict_state_requires_the_paths_it_claims() {
    // A conflicting state with nothing to print: the refusal's whole value is
    // the list, so an empty one is a modelling fault, not a refusal.
    let mut ctx = migrate_context(TreeState::Conflicting);
    ctx.would_conflict.clear();
    assert_eq!(
        plan_migration(&ctx),
        Err(MigrateFault::ConflictWithoutPaths)
    );

    // The mirror: paths computed for a state that is not conflicting.
    for state in [TreeState::Clean, TreeState::Dirty] {
        let mut ctx = migrate_context(state);
        ctx.would_conflict = PLANTED_CONFLICT_PATHS.to_vec();
        assert_eq!(
            plan_migration(&ctx),
            Err(MigrateFault::PathsWithoutConflictState { state })
        );
    }

    // And an empty printed list, if some renderer produced one anyway. The
    // per-stop `git status --short` is not a substitute for the list: it is a
    // different instruction, at a stop that has not happened yet.
    let refusal = conflict_refusal();
    let accepted = render_refusal(&refusal);
    let mut stripped = accepted.clone();
    stripped.drain(2..5);
    assert_eq!(
        audit_refusal_output(&refusal, &stripped),
        Err(RefusalReject::NoConflictPathsPrinted)
    );
}

#[test]
fn panel_migrate_dirty_tree_refusal_never_names_a_rebase() {
    let refusal = refusal_for(&migrate_context(TreeState::Dirty));
    assert_eq!(refusal, MigrateRefusal::DirtyTree);

    let output = render_refusal(&refusal);
    assert_eq!(
        audit_refusal_output(&refusal, &output),
        Ok(RefusalAudit {
            lines: 9,
            commands: 4,
            paths: 0,
        })
    );

    // The dirty tree is refused before any comparison with the fetched target,
    // so no branch of the rebase remedy belongs in this output.
    for planted in [
        PANEL_MIGRATE_REBASE,
        PANEL_MIGRATE_CONTINUE,
        PANEL_MIGRATE_ABORT,
    ] {
        let mut planted_output = output.clone();
        planted_output.push(format!("  {planted}"));
        assert_eq!(
            audit_refusal_output(&refusal, &planted_output),
            Err(RefusalReject::UnexpectedRebaseInstruction {
                command: planted.to_string(),
            })
        );
    }

    assert_eq!(
        audit_refusal_output(
            &refusal,
            &swapping(&output, PANEL_MIGRATE_STATUS, PANEL_MIGRATE_STASH_PUSH)
        ),
        Err(RefusalReject::CommandsOutOfOrder {
            earlier: PANEL_MIGRATE_STATUS.to_string(),
            later: PANEL_MIGRATE_STASH_PUSH.to_string(),
        })
    );
}

#[test]
fn panel_migrate_unpublished_migration_refuses_without_a_mutation() {
    let mut ctx = migrate_context(TreeState::Clean);
    ctx.migration_reachable = false;
    let refusal = refusal_for(&ctx);
    assert_eq!(
        refusal,
        MigrateRefusal::UnpublishedMigration {
            required: PINNED_MIGRATION,
            fetched: FETCHED_ORIGIN_V3,
        },
        "an unreachable migration is its own typed refusal, not a rebase onto the pin"
    );

    let output = render_refusal(&refusal);
    assert_eq!(
        audit_refusal_output(&refusal, &output),
        Ok(RefusalAudit {
            lines: 5,
            commands: 1,
            paths: 0,
        })
    );
    assert!(
        output
            .iter()
            .all(|line| migrate_command(line).is_none_or(|command| !command.starts_with("git "))),
        "the contributor cannot migrate to something unpublished, so this refusal offers no \
         git command at all: {output:?}"
    );
    assert!(
        !output.join("\n").contains("git rebase"),
        "and above all no rebase, onto the pin or anything else"
    );
    assert!(
        output.iter().any(|line| line.contains(PINNED_MIGRATION.0)),
        "the pin is still named - as the missing precondition, in prose"
    );

    // A planted renderer that helpfully suggests a way to get the files.
    for (planted, expected) in [
        (
            format!("git rebase {}", PINNED_MIGRATION.0),
            RefusalReject::RebaseOntoObjectName {
                target: PINNED_MIGRATION.0.to_string(),
            },
        ),
        (
            PANEL_MIGRATE_FETCH.to_string(),
            RefusalReject::GitCommandInBlockedRefusal {
                command: PANEL_MIGRATE_FETCH.to_string(),
            },
        ),
    ] {
        let mut planted_output = output.clone();
        planted_output.push(format!("  {planted}"));
        assert_eq!(
            audit_refusal_output(&refusal, &planted_output),
            Err(expected),
            "a blocked refusal that grew a git command must be rejected: {planted}"
        );
    }
}

#[test]
fn panel_migrate_target_unavailable_refuses_without_naming_a_command() {
    let refusal = unavailable_refusal();
    assert_eq!(
        refusal,
        MigrateRefusal::TargetUnavailable {
            current: CURRENT_ORIGIN_V3,
        },
        "no fetched target is a refusal, not a fallback to the pinned revision"
    );

    let output = render_refusal(&refusal);
    assert_eq!(
        audit_refusal_output(&refusal, &output),
        Ok(RefusalAudit {
            lines: 5,
            commands: 1,
            paths: 0,
        })
    );
    assert!(
        output
            .iter()
            .all(|line| migrate_command(line).is_none_or(|command| !command.starts_with("git "))),
        "there is nothing to rebase onto, so a printed sequence would be instructing a \
         contributor to run commands that cannot succeed: {output:?}"
    );

    // Every git command is rejected here, including the ones the conflict
    // refusal is required to print.
    for planted in [
        PANEL_MIGRATE_FETCH,
        PANEL_MIGRATE_REBASE,
        PANEL_MIGRATE_STATUS,
        PANEL_MIGRATE_ADD,
    ] {
        let mut planted_output = output.clone();
        planted_output.push(format!("  {planted}"));
        assert_eq!(
            audit_refusal_output(&refusal, &planted_output),
            Err(RefusalReject::GitCommandInBlockedRefusal {
                command: planted.to_string(),
            })
        );
    }

    // The refusal has to say what is missing and where this checkout stands.
    let stripped: Vec<String> = output
        .iter()
        .filter(|line| !line.contains(CURRENT_ORIGIN_V3.0))
        .cloned()
        .collect();
    assert_eq!(
        stripped.len(),
        output.len() - 1,
        "the planted fixture must actually drop the diagnosis line"
    );
    assert_eq!(
        audit_refusal_output(&refusal, &stripped),
        Err(RefusalReject::MissingDiagnosis {
            needle: CURRENT_ORIGIN_V3.0.to_string(),
        })
    );
}

#[test]
fn panel_migrate_clean_tree_lands_on_the_fetched_branch_and_never_the_pin() {
    assert_eq!(
        plan_migration(&migrate_context(TreeState::Clean)),
        Ok(MigrateOutcome::Rebase {
            target_ref: PANEL_MIGRATE_TARGET_REF,
            from: CURRENT_ORIGIN_V3,
            onto: FETCHED_ORIGIN_V3,
        }),
        "the branch moves onto the fetched protected branch: forward, and never onto the pin"
    );
}

#[test]
fn panel_migrate_refusal_corpus_is_counted_and_non_empty() {
    // The planted object names are really 40-hex, so the rejections above are
    // rejections of the instruction that ships, not of a malformed fixture.
    for name in [CURRENT_ORIGIN_V3, FETCHED_ORIGIN_V3, PINNED_MIGRATION] {
        assert!(
            is_object_name(name.0),
            "{} is not a 40-hex object name",
            name.0
        );
    }

    let corpus = migrate_corpus();
    assert_eq!(
        corpus.len(),
        5,
        "the wrapper is modelled on five paths: one that proceeds and four typed refusals"
    );

    let mut proceeded = 0usize;
    let mut audits: Vec<(&str, RefusalAudit)> = Vec::new();
    for (label, ctx) in &corpus {
        match plan_migration(ctx).unwrap_or_else(|fault| {
            panic!("the modelled context is self-consistent ({label}): {fault:?}")
        }) {
            MigrateOutcome::Rebase { target_ref, .. } => {
                assert_eq!(target_ref, PANEL_MIGRATE_TARGET_REF);
                proceeded += 1;
            }
            MigrateOutcome::Refuse(refusal) => {
                let audit = audit_refusal_output(&refusal, &render_refusal(&refusal))
                    .unwrap_or_else(|reject| {
                        panic!("the modelled refusal renders acceptably ({label}): {reject:?}")
                    });
                audits.push((label, audit));
            }
        }
    }

    assert_eq!(proceeded, 1, "only the clean tree proceeds");
    assert!(
        !audits.is_empty(),
        "a corpus that examined nothing is not evidence"
    );
    assert_eq!(
        audits,
        vec![
            (
                "dirty tree",
                RefusalAudit {
                    lines: 9,
                    commands: 4,
                    paths: 0,
                },
            ),
            (
                "conflicting update",
                RefusalAudit {
                    lines: 17,
                    commands: 7,
                    paths: 3,
                },
            ),
            (
                "unpublished migration",
                RefusalAudit {
                    lines: 5,
                    commands: 1,
                    paths: 0,
                },
            ),
            (
                "target unavailable",
                RefusalAudit {
                    lines: 5,
                    commands: 1,
                    paths: 0,
                },
            ),
        ],
        "every modelled refusal is rendered and audited; a shrinking corpus is a scan that \
         examined less than it claims"
    );
    assert_eq!(
        audits
            .iter()
            .map(|(_, audit)| audit.commands)
            .sum::<usize>(),
        13,
        "the audited command total: 7 for the conflict refusal, 4 for the dirty tree, and the \
         rerun for each blocked one"
    );
    assert_eq!(
        audits.iter().map(|(_, audit)| audit.paths).sum::<usize>(),
        PLANTED_CONFLICT_PATHS.len(),
        "the predicted paths are printed exactly once, by the one refusal that computed them"
    );
}
