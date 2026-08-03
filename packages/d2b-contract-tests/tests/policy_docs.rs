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

use std::collections::BTreeSet;

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

fn execution_manifest_completed_leaf_enum(schema: &Value, target: &str) -> BTreeSet<String> {
    let clauses = schema
        .get("allOf")
        .and_then(Value::as_array)
        .expect("execution-manifest schema allOf");
    let matching: Vec<&Value> = clauses
        .iter()
        .filter(|clause| {
            clause
                .pointer("/if/properties/target/const")
                .and_then(Value::as_str)
                == Some(target)
        })
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "execution-manifest-policy: expected one completed-leaf rule for {target}"
    );
    matching[0]
        .pointer("/then/properties/completed_leaves/items/enum")
        .and_then(Value::as_array)
        .expect("execution-manifest-policy: completed-leaf enum is missing")
        .iter()
        .map(|value| {
            value
                .as_str()
                .expect("execution-manifest-policy: completed-leaf enum value is a string")
                .to_string()
        })
        .collect()
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
    let nix_driver = read_repo_file("tests/test-nix-unit.sh");
    let tests_readme = read_repo_file("tests/README.md");
    let tests_agents = read_repo_file("tests/AGENTS.md");
    let changelog = read_repo_file("changelog.d/test-orchestration-speed.md");
    let benchmark = read_repo_file("specs/002-optimize-test-orchestration/benchmark-results.md");
    let nix_jobs = read_repo_file("tests/unit/nix/eval-jobs.nix");
    let flake = read_repo_file("flake.nix");
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

    let nix_unit_baseline_leaves = [
        "nix-unit",
        "nix-unit-daemon",
        "nix-unit-guest",
        "nix-unit-misc",
        "nix-unit-network",
        "nix-unit-runtime",
        "nix-unit-state",
    ];
    let expected_nix_unit_leaves: BTreeSet<String> = nix_unit_baseline_leaves
        .iter()
        .map(|leaf| (*leaf).to_string())
        .collect();
    let schema_nix_unit_leaves = execution_manifest_completed_leaf_enum(&schema, "test-nix-unit");
    assert_eq!(
        schema_nix_unit_leaves, expected_nix_unit_leaves,
        "execution-manifest-policy: Nix-unit completed-leaf enum drifted"
    );
    for leaf in nix_unit_baseline_leaves {
        assert!(
            nix_driver.contains(leaf),
            "execution-manifest-policy: Nix-unit emitter does not name baseline leaf {leaf}"
        );
        assert!(
            prose.contains(&format!("`{leaf}`")),
            "execution-manifest-policy: reference prose does not document Nix-unit leaf {leaf}"
        );
    }
    assert!(
        nix_driver.contains("nix_unit_baseline_leaves=("),
        "execution-manifest-policy: Nix-unit emitter must publish a fixed baseline leaf set"
    );
    assert!(
        nix_driver.contains("nix-eval-jobs")
            && nix_driver.contains("--no-instantiate")
            && nix_driver.contains("nixUnitJobs"),
        "execution-manifest-policy: Nix-unit emitter must use the evaluation-only nix-eval-jobs surface"
    );
    assert!(
        nix_jobs.contains("builtins.tryEval")
            && nix_jobs.contains("jobsFor")
            && flake.contains("nixUnitJobs = forAllSystems"),
        "execution-manifest-policy: Nix-unit corpus and jobs attrset are not wired together"
    );
    for marker in [
        "nix-eval-jobs",
        "--no-instantiate",
        "nixUnitJobs.<system>",
        "installables",
        "realized_checks",
        "893",
    ] {
        assert!(
            prose.contains(marker),
            "execution-manifest-policy: Nix-unit reference prose is missing emitter marker {marker}"
        );
    }
    assert!(
        flake.contains("nix-unit = pkgs.mkShellNoCC")
            && flake.contains("nix-eval-jobs")
            && flake.contains("jq"),
        "execution-manifest-policy: focused locked Nix-unit dev shell is missing"
    );
    assert!(
        nix_driver.contains("D2B_NIX_UNIT_WORKERS")
            && nix_driver.contains("D2B_NIX_UNIT_MEMORY_MB")
            && !nix_driver.contains("D2B_NIX_EVAL_JOBS_WORKERS")
            && !nix_driver.contains("D2B_NIX_EVAL_JOBS_MEMORY_MB")
            && nix_driver.contains("cpu_cap")
            && nix_driver.contains("memory_cap")
            && nix_driver.contains("--workers")
            && nix_driver.contains("--max-memory-size")
            && nix_driver.contains("4096"),
        "execution-manifest-policy: Nix-unit resource controls are incomplete"
    );
    assert!(
        nix_driver.contains("reserve_mb=3072")
            && nix_driver.contains("worker_budget_mb=$((memory_mb + 2048))"),
        "execution-manifest-policy: Nix-unit hosted-runner memory envelope drifted"
    );
    assert!(
        nix_driver.contains("D2B_NIX_UNIT_JOBS is retired")
            && !nix_driver.contains("${D2B_NIX_UNIT_JOBS:-"),
        "execution-manifest-policy: retired Nix-unit worker knob is still accepted"
    );
    let retired_knob = nix_driver
        .find("D2B_NIX_UNIT_JOBS is retired")
        .expect("execution-manifest-policy: retired knob diagnostic is missing");
    assert!(
        nix_driver[retired_knob..].contains("exit 2"),
        "execution-manifest-policy: retired Nix-unit worker knob must exit 2"
    );
    assert!(
        nix_driver.contains("D2B_NIX_UNIT_WORKERS")
            && !nix_driver.contains("D2B_NIX_EVAL_JOBS_WORKERS"),
        "execution-manifest-policy: retired knob remedy must name the operator-intent worker control"
    );
    for (label, doc) in [
        ("tests README", tests_readme.as_str()),
        ("tests AGENTS", tests_agents.as_str()),
        ("changelog", changelog.as_str()),
        ("benchmark", benchmark.as_str()),
    ] {
        assert!(
            doc.contains("D2B_NIX_UNIT_WORKERS")
                && doc.contains("D2B_NIX_UNIT_MEMORY_MB")
                && !doc.contains("D2B_NIX_EVAL_JOBS_WORKERS")
                && !doc.contains("D2B_NIX_EVAL_JOBS_MEMORY_MB"),
            "execution-manifest-policy: {label} has stale implementation-specific Nix-unit resource knobs"
        );
    }
    assert!(
        nix_driver.contains("if ! publish_manifest_fragment \"$nix_unit_surface\" failed; then")
            && !nix_driver
                .contains("if publish_manifest_fragment \"$nix_unit_surface\" failed; then")
            && nix_driver.contains("local rc=$?")
            && nix_driver.contains("exit \"$rc\"")
            && nix_driver.contains("nix_unit_command_succeeded"),
        "execution-manifest-policy: Nix-unit EXIT trap must diagnose only failed publication and preserve status/evidence semantics"
    );
    assert!(
        !nix_driver.contains("cat \"$result_file\""),
        "execution-manifest-policy: Nix-unit full runs must not dump raw JSONL results"
    );
    assert!(
        nix_driver.contains("2>\"$tool_stderr\"")
            && nix_driver.contains("emit_sanitized_tool_stderr()")
            && nix_driver.contains("while IFS= read -r line || [ -n \"$line\" ]; do")
            && nix_driver.contains("line=${line//\"$flake_root\"/<repo>}")
            && nix_driver.contains("line=${line//\"$HOME\"/<home>}"),
        "execution-manifest-policy: evaluator stderr must be captured and path-sanitized"
    );
    assert!(
        nix_driver.contains("flake_label=d2b"),
        "execution-manifest-policy: Nix-unit progress must use a fixed path-free flake label"
    );
    for line in nix_driver.lines() {
        if line.contains("log ") && line.to_ascii_lowercase().contains("flake") {
            assert!(
                !line.contains("flake_ref"),
                "execution-manifest-policy: path-bearing flake reference appears in progress log: {line}"
            );
        }
    }
    let failure_reporting = nix_driver
        .split("for failure in")
        .nth(1)
        .and_then(|region| region.split("done").next())
        .expect("execution-manifest-policy: Nix-unit failure reporting loop is missing");
    assert!(
        failure_reporting.contains("failure=${failure//\"$flake_root\"/<repo>}")
            && failure_reporting.contains(">&2")
            && !failure_reporting.contains("log "),
        "execution-manifest-policy: evaluator failures must be root-sanitized and printed directly to stderr"
    );
    for marker in [
        "expected_cases_file",
        "actual_cases_file",
        "missing_cases",
        "unexpected_cases",
        "comm -23",
        "comm -13",
        "if ! jq -r ",
        "if ! comm -23 ",
        "if ! comm -13 ",
        "failures_file",
        "missing_cases_file",
        "unexpected_cases_file",
        "sort -u",
        "missing evaluated case",
        "unexpected evaluated case",
        "run make nix-unit-pin",
        "case_names_ok",
        "select(. != \"__nix_unit_integrity\"",
        "result_count=$(jq -s 'length'",
        "case_count=$((result_count - integrity_count))",
        "integrity_count",
    ] {
        assert!(
            nix_driver.contains(marker),
            "execution-manifest-policy: Nix-unit pin/integrity diagnostic marker is missing: {marker}"
        );
    }
    assert!(
        nix_driver.contains("|| [ \"$case_names_ok\" -ne 1 ]; then"),
        "execution-manifest-policy: Nix-unit must fail on symmetric-difference drift even when counts match"
    );

    // Lifecycle entry must precede every Nix evaluator, runner, or toolchain
    // process. Ignore comments so the ordering check follows executable text.
    let executable_nix_driver = nix_driver
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    let lifecycle_index = executable_nix_driver
        .find("execution-manifest.pl\" run")
        .expect("execution-manifest-policy: Nix-unit lifecycle entry is missing");
    let first_nix_process = Regex::new(r"\bnix\s+(?:eval|develop|build|flake)\b|\bnix-eval-jobs\b")
        .expect("valid Nix-unit lifecycle ordering regex")
        .find(&executable_nix_driver)
        .expect("execution-manifest-policy: Nix-unit evaluator or runner is missing")
        .start();
    assert!(
        lifecycle_index < first_nix_process,
        "execution-manifest-policy: Nix-unit lifecycle must begin before Nix evaluation or runner entry"
    );
    assert!(
        nix_driver.contains("D2B_NIX_UNIT_MANIFEST_LIFECYCLE=1")
            && nix_driver.contains("D2B_NIX_UNIT_TOOLCHAIN_REENTRY"),
        "execution-manifest-policy: Nix-unit lifecycle or toolchain re-entry guard is missing"
    );
}
