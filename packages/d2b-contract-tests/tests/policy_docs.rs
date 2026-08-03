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

/// Repo-relative paths of every Markdown file under `docs/contributing/`.
fn contributing_docs() -> Vec<String> {
    let dir = d2b_contract_tests::repo_root().join("docs/contributing");
    let mut out: Vec<String> = std::fs::read_dir(&dir)
        .expect("docs/contributing must exist; AGENTS.md routes to it")
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().to_string_lossy().into_owned();
            name.ends_with(".md")
                .then(|| format!("docs/contributing/{name}"))
        })
        .collect();
    out.sort();
    assert!(
        !out.is_empty(),
        "docs/contributing must contain the process docs AGENTS.md routes to"
    );
    out
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
// one. What lands now is the *algorithm* and the corpus that proves it works:
// an exhaustive recursive census over a type graph, plus planted fixtures that
// it must accept and reject today.
//
// The rule the census enforces: every field of every struct, and every variant
// and every variant-field of every enum, reachable from the entry type, at any
// depth, must resolve to a member of the closed approved set - a redacting
// newtype, a closed enum whose own variant-fields satisfy the same rule, a
// bounded numeric, a version or stage newtype, or a collection of safe items.
// Raw text, a path, an unresolved type, and a cycle it cannot traverse to a
// fixed point are all failures. Enums are not leaves: a census that stops at
// variant names, or that descends one level into structs only, is the census
// that misses the field that leaks.
//
// **Wiring contract for the implementation commit.** The corpus below is a
// hand-written model, deliberately shaped like the metadata a real census has:
// named types, fields, variants, variant-fields, and type references resolved
// through one map. The commit that introduces the real panel receipt error
// enum builds a `TypeCorpus` from that type's actual metadata and calls
// `census_reachable_types` on it - the same predicate, not a second one - and
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

/// What the census examined. Reported, not asserted away: a census that cannot
/// say how much it looked at cannot be shown to have looked at anything.
#[derive(Debug, Default, PartialEq, Eq)]
struct CensusReport {
    types: usize,
    variants: usize,
    fields: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum CensusReject {
    /// Nothing to traverse at all.
    EmptyCorpus,
    /// The entry type resolved but reaches no variant and no field.
    NothingExamined { root: String },
    /// A referenced type the census cannot resolve. Not a skip.
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

struct Census<'a> {
    corpus: &'a TypeCorpus,
    report: CensusReport,
    /// Types fully traversed. A shared (diamond) reference is supported and
    /// counted once, so the counts stay deterministic.
    settled: BTreeSet<&'static str>,
    /// Types currently being traversed, innermost last.
    stack: Vec<&'static str>,
}

impl Census<'_> {
    fn visit(&mut self, type_name: &'static str, path: &str) -> Result<(), CensusReject> {
        if self.stack.contains(&type_name) {
            return Err(CensusReject::UnsupportedCycle {
                type_name: type_name.to_string(),
                path: path.to_string(),
            });
        }
        if self.settled.contains(type_name) {
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
        self.report.types += 1;
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
                    self.report.fields += 1;
                    self.visit(field_type, &format!("{path}.{field}"))?;
                }
            }
            TypeDef::Enum(variants) => {
                for (variant, fields) in variants {
                    self.report.variants += 1;
                    for &(field, field_type) in fields {
                        self.report.fields += 1;
                        self.visit(field_type, &format!("{path}::{variant}.{field}"))?;
                    }
                }
            }
            TypeDef::Collection(item) => {
                self.visit(item, &format!("{path}[]"))?;
            }
        }
        self.stack.pop();
        self.settled.insert(type_name);
        Ok(())
    }
}

/// Traverse every type reachable from `root` and prove each member is safe.
///
/// This is a fixture predicate, not a production scan: it reads the modelled
/// `corpus` it is handed. The implementation commit that adds the real panel
/// receipt error enum is expected to build that corpus from the type's own
/// metadata and call this function, rather than writing a second census.
fn census_reachable_types(
    corpus: &TypeCorpus,
    root: &'static str,
) -> Result<CensusReport, CensusReject> {
    if corpus.is_empty() {
        return Err(CensusReject::EmptyCorpus);
    }
    let mut census = Census {
        corpus,
        report: CensusReport::default(),
        settled: BTreeSet::new(),
        stack: Vec::new(),
    };
    census.visit(root, root)?;
    if census.report.variants == 0 && census.report.fields == 0 {
        return Err(CensusReject::NothingExamined {
            root: root.to_string(),
        });
    }
    Ok(census.report)
}

const CENSUS_ROOT: &str = "PanelReceiptError";

/// The accepted fixture: an entry enum whose reachable graph is entirely
/// approved. It exercises every supported shape - variant fields, a fieldless
/// variant, a nested struct, a closed fieldless enum, a collection of typed
/// items whose own variant-fields are safe, and a type (`RemedyPlan`) reached
/// from two different variants.
fn safe_panel_receipt_corpus() -> TypeCorpus {
    TypeCorpus::from([
        (
            "PanelReceiptError",
            TypeDef::Enum(vec![
                (
                    "HarnessVersionUnparseable",
                    vec![("observed", "BannerDigest"), ("stage", "ReceiptStage")],
                ),
                (
                    "HarnessUnavailable",
                    vec![("attempts", "AttemptCount"), ("plan", "RemedyPlan")],
                ),
                (
                    "ReceiptRejected",
                    vec![("alias", "CorrelationAlias"), ("plan", "RemedyPlan")],
                ),
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

/// The entry enum with one extra variant appended, for planting a defect at
/// the top level of the reachable graph.
fn root_enum_with_extra_variant(variant: &'static str, fields: FieldList) -> TypeDef {
    let TypeDef::Enum(mut variants) = safe_panel_receipt_corpus()
        .get(CENSUS_ROOT)
        .expect("the fixture corpus defines its entry type")
        .clone()
    else {
        panic!("the fixture entry type is an enum");
    };
    variants.push((variant, fields));
    TypeDef::Enum(variants)
}

#[test]
fn safe_type_census_accepts_the_approved_fixture() {
    let report = census_reachable_types(&safe_panel_receipt_corpus(), CENSUS_ROOT)
        .expect("the approved fixture corpus is accepted");

    // Non-vacuous counts: 12 reachable types, 11 variants (4 on the entry
    // enum, 3 on the closed fieldless enum, 4 on RemedyAction), 15 fields.
    // `RemedyPlan` is reached from two variants and counted once.
    assert_eq!(
        report,
        CensusReport {
            types: 12,
            variants: 11,
            fields: 15,
        },
        "the census must report what it examined; a drifting count means the traversal changed"
    );
}

#[test]
fn safe_type_census_rejects_planted_unsafe_members() {
    // A raw String at the top level, carrying no protected marking at all.
    assert_eq!(
        census_reachable_types(
            &corpus_with(&[
                (
                    CENSUS_ROOT,
                    root_enum_with_extra_variant("Diagnostic", vec![("message", "RawMessage")])
                ),
                ("RawMessage", TypeDef::Leaf(LeafKind::RawText)),
            ]),
            CENSUS_ROOT
        ),
        Err(CensusReject::UnapprovedLeaf {
            kind: LeafKind::RawText,
            type_name: "RawMessage".to_string(),
            path: "PanelReceiptError::Diagnostic.message".to_string(),
        })
    );

    // A PathBuf at the top level.
    assert_eq!(
        census_reachable_types(
            &corpus_with(&[
                (
                    CENSUS_ROOT,
                    root_enum_with_extra_variant("ReceiptMissing", vec![("path", "ReceiptPath")])
                ),
                ("ReceiptPath", TypeDef::Leaf(LeafKind::RawPath)),
            ]),
            CENSUS_ROOT
        ),
        Err(CensusReject::UnapprovedLeaf {
            kind: LeafKind::RawPath,
            type_name: "ReceiptPath".to_string(),
            path: "PanelReceiptError::ReceiptMissing.path".to_string(),
        })
    );

    // A raw String on a struct field two levels below the entry type
    // (entry enum -> RemedyPlan -> ProducerContext). A census that inspects
    // only the top level passes this fixture.
    assert_eq!(
        census_reachable_types(
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
            CENSUS_ROOT
        ),
        Err(CensusReject::UnapprovedLeaf {
            kind: LeafKind::RawText,
            type_name: "RawMessage".to_string(),
            path: "PanelReceiptError::HarnessUnavailable.plan.producer.note".to_string(),
        })
    );

    // A raw String on an enum variant-field two levels below the entry type,
    // reached through another enum. A census that stops at variant names
    // passes this fixture.
    assert_eq!(
        census_reachable_types(
            &corpus_with(&[
                (
                    CENSUS_ROOT,
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
            CENSUS_ROOT
        ),
        Err(CensusReject::UnapprovedLeaf {
            kind: LeafKind::RawText,
            type_name: "RawMessage".to_string(),
            path: "PanelReceiptError::Escalated.inner::Detail.evidence".to_string(),
        })
    );

    // A path on a variant-field of the typed action collection: entry enum ->
    // RemedyPlan -> collection -> RemedyAction variant.
    assert_eq!(
        census_reachable_types(
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
            CENSUS_ROOT
        ),
        Err(CensusReject::UnapprovedLeaf {
            kind: LeafKind::RawPath,
            type_name: "ReceiptPath".to_string(),
            path: "PanelReceiptError::HarnessUnavailable.plan.actions[]::RerunPreflight.workdir"
                .to_string(),
        })
    );
}

#[test]
fn safe_type_census_fails_closed_on_unresolved_cyclic_and_empty_corpora() {
    // A type the census does not recognise. Unresolved is a failure: a census
    // that skips what it cannot resolve has counted the easy fields.
    assert_eq!(
        census_reachable_types(
            &corpus_with(&[(
                CENSUS_ROOT,
                root_enum_with_extra_variant("Opaque", vec![("payload", "UnmodelledPayload")])
            )]),
            CENSUS_ROOT
        ),
        Err(CensusReject::Unresolved {
            type_name: "UnmodelledPayload".to_string(),
            path: "PanelReceiptError::Opaque.payload".to_string(),
        })
    );

    // A cycle the census cannot traverse to a fixed point.
    assert_eq!(
        census_reachable_types(
            &corpus_with(&[(
                "ProducerContext",
                TypeDef::Struct(vec![("stage", "PipelineStage"), ("parent", "RemedyPlan"),])
            )]),
            CENSUS_ROOT
        ),
        Err(CensusReject::UnsupportedCycle {
            type_name: "RemedyPlan".to_string(),
            path: "PanelReceiptError::HarnessUnavailable.plan.producer.parent".to_string(),
        })
    );

    // An entry enum with no variants: nothing was examined, so nothing was
    // shown safe.
    assert_eq!(
        census_reachable_types(
            &corpus_with(&[(CENSUS_ROOT, TypeDef::Enum(vec![]))]),
            CENSUS_ROOT
        ),
        Err(CensusReject::NothingExamined {
            root: CENSUS_ROOT.to_string(),
        })
    );

    // An empty corpus.
    assert_eq!(
        census_reachable_types(&TypeCorpus::new(), CENSUS_ROOT),
        Err(CensusReject::EmptyCorpus)
    );

    // An entry type that is not in the corpus at all.
    assert_eq!(
        census_reachable_types(&safe_panel_receipt_corpus(), "AbsentEntryType"),
        Err(CensusReject::Unresolved {
            type_name: "AbsentEntryType".to_string(),
            path: "AbsentEntryType".to_string(),
        })
    );
}
