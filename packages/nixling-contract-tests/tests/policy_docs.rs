//! Docs / AGENTS / CLI-manpage / kernel-module-matrix policy lints (the
//! "H-group"), migrated from the `tests/*-eval.sh` bash gates. Each test reads
//! the real repo files (via the `nixling_contract_tests` repo-file helpers) and
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

use nixling_contract_tests::{read_repo_file, repo_path_exists};
use regex::Regex;

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
//   * Positive invariants — the rewrite must surface the daemon-only end-state
//     section explicitly, cross-reference ADR 0015, and mention nixlingd /
//     nixling-priv-broker.socket / SpawnRunner.
//   * Negative invariants — a per-line scan: any line matching a forbidden
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
        any_line_matches(&agents, r"nixlingd"),
        "AGENTS.md does not mention nixlingd"
    );
    assert!(
        any_line_matches(&agents, r"nixling-priv-broker\.socket"),
        "AGENTS.md does not mention nixling-priv-broker.socket (socket-activation contract)"
    );
    assert!(
        any_line_matches(&agents, r"SpawnRunner"),
        "AGENTS.md does not describe broker SpawnRunner for TPM/USBIP/GPU rewire"
    );

    // --- Negative invariants (per-line forbidden scan w/ allowed marker) --
    //
    // `forbidden_re` (grep -nE, case-sensitive) and `allowed_marker_re`
    // (grep -qEi, case-insensitive) are ported verbatim from the bash gate.
    // The forbidden alternation targets the canonical legacy-as-live shapes:
    // nixling@<vm> / microvm@<vm> per-VM systemd templates, retired
    // host-singleton framework services, microvms.target, the legacy bash-CLI
    // opt-in knobs, and the "bash CLI" phrase. A line keeps its forbidden
    // pattern only when it ALSO mentions a historical / migration / retired
    // marker (it is describing the deletion itself).
    let forbidden_re = Regex::new(
        r"nixling@<vm>|nixling@\$\{name\}|nixling@sys-|microvm@<vm>|microvm-virtiofsd@|microvm-set-booted@|microvm-tap-interfaces@|microvm-macvtap-interfaces@|microvm-pci-devices@|nixling-<vm>-(gpu|snd|video|swtpm|store-sync)\.service|nixling-sys-<env>-usbipd|nixling-otel-relay@|nixling-known-hosts-refresh@|nixling-vfsd-watchdog@|nixling-ch-exporter\.service|nixling-otel-host-bridge\.service|nixling-net-route-preflight\.service|nixling-audit-check\.(service|timer)|microvms\.target|NIXLING_LEGACY_BASH_OPT_IN|NIXLING_LEGACY_CLI|\bbash CLI\b",
    )
    .expect("valid forbidden regex");
    let allowed_marker_re = Regex::new(
        r"(?i)retired|removed|deleted|legacy|historical|no longer|no per-|no per-VM|end-state|P6|pre-v1|v0\.4|ADR 0015|denylist|ph6-|rewire|rewritten|migration|supersedes|reintroduce|Don't|There is no|not mention|moved into|either moved|fail-closed|-style",
    )
    .expect("valid allowed-marker regex");

    let mut violations: Vec<String> = Vec::new();
    for (idx, line) in agents.lines().enumerate() {
        if !forbidden_re.is_match(line) {
            continue;
        }
        if allowed_marker_re.is_match(line) {
            continue;
        }
        violations.push(format!(
            "AGENTS.md:{} describes a retired surface as live (no historical marker): {line}",
            idx + 1
        ));
    }

    assert!(
        violations.is_empty(),
        "agents-md-rewrite-eval: {} line(s) describe retired surfaces as live; see ADR 0015:\n{}",
        violations.len(),
        violations.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Migrated from tests/manpage-completeness-eval.sh.
//
// Asserts that every top-level clap subcommand declared in
// `packages/nixling/src/lib.rs` (`enum NativeCommand { ... }`) is documented as
// a section in the committed nixling(1) manpage at `docs/manpages/nixling.1`.
// clap_mangen emits one `.TP` entry per subcommand under the SUBCOMMANDS block
// (rendered as `nixling-<name>(1)`); a new verb that lands without rerunning
// `cargo xtask gen-cli-shell-artifacts` silently drops out of the manpage. This
// gate fails closed on that drift without needing a cargo toolchain.
// ---------------------------------------------------------------------------
#[test]
fn manpage_documents_every_top_level_subcommand() {
    let cli_rel = "packages/nixling/src/lib.rs";
    let manpage_rel = "docs/manpages/nixling.1";
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
/// `nixling\-` prefix are reduced to their bare `<name>` by stripping the
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
        if in_sub {
            if let Some(rest) = line.strip_prefix("nixling\\-") {
                let rest = rest.strip_suffix("(1)").unwrap_or(rest);
                out.insert(rest.replace("\\-", "-"));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Migrated from tests/kernel-module-matrix-eval.sh (matrix-parity half).
//
// Asserts the REQUIRED / OPTIONAL module constants in
// `packages/nixlingd/src/kernel_module_check.rs` stay in sync with the
// operator-reference matrix in `docs/reference/kernel-module-check.md`. The
// source must carry each canonical `"<module>"` string literal, the doc must
// cite each module backticked, and the source must declare each canonical
// `pub const <IDENT>` (so a stealth refactor that renames a constant surfaces).
// ---------------------------------------------------------------------------
#[test]
fn kernel_module_matrix_source_doc_parity() {
    let src_rel = "packages/nixlingd/src/kernel_module_check.rs";
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
// Migrated from tests/kernel-module-matrix-eval.sh (typed-error contract half).
//
// Asserts the fatal-typed-error contract: `packages/nixlingd/src/typed_error.rs`
// carries the `HostKernelModulesMissing` variant at exit code 64 with kind
// "host-kernel-modules-missing".
// ---------------------------------------------------------------------------
#[test]
fn kernel_module_missing_typed_error_contract() {
    let typed_rel = "packages/nixlingd/src/typed_error.rs";
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
