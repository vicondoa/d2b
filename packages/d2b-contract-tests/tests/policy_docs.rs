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

use std::{
    collections::BTreeSet,
    fmt, fs,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

use d2b_contract_tests::{read_repo_file, repo_path_exists};
use regex::Regex;
use serde_json::Value;

const REDACTED: &str = "<redacted>";

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
#[derive(PartialEq, Eq)]
enum DirScanFault {
    /// A `read_dir` entry could not be read. An unreadable entry is a doc the
    /// scan did not look at, and a doc the scan did not look at is not a doc
    /// the scan cleared.
    UnreadableEntry { dir: String, detail: String },
    /// The directory holds no Markdown file at all, so the scan below it would
    /// pass by having nothing to scan.
    NoMarkdownFiles { dir: String },
}

/// Variant only. This fault carries the directory it was scanning and the
/// operating system's message about the entry that failed, and it is printed by
/// the panic that aborts the gate, so the payload does not travel with the
/// name. Equality still compares it, so the fail-closed assertions below
/// discriminate on the directory and the detail exactly as they did.
impl fmt::Debug for DirScanFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let variant = match self {
            Self::UnreadableEntry { .. } => "UnreadableEntry",
            Self::NoMarkdownFiles { .. } => "NoMarkdownFiles",
        };
        write!(f, "DirScanFault::{variant}({REDACTED})")
    }
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

#[test]
fn contributing_doc_enumeration_fault_debug_redacts_the_directory_and_the_detail() {
    // The fault is printed by the panic that aborts the gate. That message is a
    // failure report, not a place to widen what a doc scan discloses about the
    // checkout it ran in, so `Debug` names the variant and nothing else.
    for (fault, planted) in [
        (
            DirScanFault::UnreadableEntry {
                dir: CONTRIBUTING_DIR.to_string(),
                detail: "workflow.md: denied".to_string(),
            },
            "workflow.md",
        ),
        (
            DirScanFault::NoMarkdownFiles {
                dir: CONTRIBUTING_DIR.to_string(),
            },
            CONTRIBUTING_DIR,
        ),
    ] {
        let rendered = format!("{fault:?}");
        assert!(
            !rendered.contains(planted) && !rendered.contains(CONTRIBUTING_DIR),
            "a scan fault's Debug must not carry `{planted}`: {rendered}"
        );
        assert!(
            rendered.ends_with(&format!("({REDACTED})")),
            "a scan fault's Debug redacts its payload: {rendered}"
        );
    }

    // Redacting the payload does not stop equality from discriminating on it.
    assert_ne!(
        DirScanFault::NoMarkdownFiles {
            dir: CONTRIBUTING_DIR.to_string(),
        },
        DirScanFault::NoMarkdownFiles {
            dir: "docs/reference".to_string(),
        }
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
    const BUDGET: usize = 20_000;
    let bytes = read_repo_file("AGENTS.md").len();
    assert!(
        bytes <= BUDGET,
        "AGENTS.md is {bytes} bytes, over its {BUDGET}-byte budget. It is loaded into every \
         agent session on every turn. Move detail into docs/contributing/ and leave a rule \
         plus a link, rather than raising the budget."
    );
}

fn markdown_anchor_slug(heading: &str) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for ch in heading.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch.is_whitespace() {
            pending_dash = true;
        }
    }
    slug
}

fn markdown_has_anchor(content: &str, anchor: &str) -> bool {
    let expected = anchor.trim_start_matches('#').to_ascii_lowercase();
    content.lines().any(|line| {
        let heading = line.trim_start_matches('#');
        line.starts_with('#') && markdown_anchor_slug(heading) == expected
    }) || content.contains(&format!("id=\"{expected}\""))
        || content.contains(&format!("id='{expected}'"))
}

fn markdown_repo_path_is_contained(path: &str) -> bool {
    path.is_empty()
        || (!path.contains(':')
            && !Path::new(path).is_absolute()
            && Path::new(path)
                .components()
                .all(|component| matches!(component, Component::CurDir | Component::Normal(_))))
}

fn markdown_link_is_external(target: &str) -> bool {
    target.starts_with("mailto:")
        || target.starts_with("//")
        || target.starts_with("http://")
        || target.starts_with("https://")
}

fn resolved_repo_path_is_contained(root: &Path, path: &str) -> bool {
    let canonical_root = fs::canonicalize(root)
        .unwrap_or_else(|error| panic!("cannot resolve repo root {}: {error}", root.display()));
    fs::canonicalize(root.join(path.trim_start_matches("./")))
        .map(|target| target.starts_with(canonical_root))
        .unwrap_or(false)
}

fn markdown_uses_reference_link_syntax(content: &str) -> bool {
    let reference_use = Regex::new(r"\]\s*\[[^]]*\]").expect("valid reference-style link regex");
    let reference_definition =
        Regex::new(r"(?m)^[ \t]{0,3}\[[^]]+\]:").expect("valid link definition regex");
    reference_use.is_match(content) || reference_definition.is_match(content)
}

#[test]
fn markdown_repo_paths_reject_absolute_and_parent_escape() {
    for invalid in [
        "/etc/passwd",
        "../outside.md",
        "docs/../README.md",
        "file:///etc/passwd",
        "C:/Windows/System32",
    ] {
        assert!(
            !markdown_repo_path_is_contained(invalid),
            "repository Markdown path must reject escape: {invalid}"
        );
    }
    for valid in [
        "",
        "README.md",
        "./README.md",
        "docs/contributing/workflow.md",
    ] {
        assert!(
            markdown_repo_path_is_contained(valid),
            "repository Markdown path must accept contained target: {valid}"
        );
    }
}

#[test]
fn markdown_repo_paths_reject_symlink_escape() {
    let tempdir = std::env::var_os("CARGO_TARGET_TMPDIR")
        .or_else(|| std::env::var_os("TEST_TMPDIR"))
        .or_else(|| std::env::var_os("TMPDIR"))
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| std::env::current_dir().expect("current directory"));
    let root = tempdir.join("policy-doc-links");
    let outside = tempdir.join("policy-doc-outside.md");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("docs")).expect("create Markdown link fixture root");
    fs::write(&outside, "outside\n").expect("write outside Markdown fixture");
    std::os::unix::fs::symlink(&outside, root.join("escape.md"))
        .expect("create escaping Markdown symlink");
    std::os::unix::fs::symlink(&outside, root.join("docs/README.md"))
        .expect("create escaping Markdown directory index");

    assert!(
        !resolved_repo_path_is_contained(&root, "escape.md"),
        "repository Markdown path must reject a symlink resolving outside its root"
    );
    assert!(
        !resolved_repo_path_is_contained(&root, "docs/README.md"),
        "repository Markdown directory index must remain inside its root"
    );
}

#[test]
fn markdown_reference_links_are_rejected_in_all_forms() {
    for invalid in [
        "[Workflow][guide]\n\n[guide]: docs/contributing/workflow.md\n",
        "[Workflow][]\n\n[Workflow]: docs/contributing/workflow.md\n",
        "[Workflow]\n\n[Workflow]: ../outside.md\n",
        "[Workflow]:\n../outside.md\n\nSee [Workflow].\n",
    ] {
        assert!(
            markdown_uses_reference_link_syntax(invalid),
            "reference-style Markdown link must be detected: {invalid}"
        );
    }
    assert!(
        !markdown_uses_reference_link_syntax("[Workflow](docs/contributing/workflow.md)\n"),
        "inline Markdown link must remain supported"
    );
}

/// A router whose links rot is worse than the monolith it replaced: the rule
/// looks documented while the detail is unreachable. Validate local Markdown
/// paths in all supported forms and validate anchors in the linked document.
#[test]
fn agents_md_routes_to_paths_that_exist() {
    let agents = read_repo_file("AGENTS.md");
    assert!(
        !markdown_uses_reference_link_syntax(&agents),
        "AGENTS.md must use inline Markdown links so every destination is validated"
    );
    let link_re = Regex::new(r"\]\(([^)\s]+)").expect("valid link regex");
    let mut missing: Vec<String> = Vec::new();
    for caps in link_re.captures_iter(&agents) {
        let target = &caps[1];
        if markdown_link_is_external(target) {
            continue;
        }

        let (path, anchor) = target.split_once('#').unwrap_or((target, ""));
        if !markdown_repo_path_is_contained(path) {
            missing.push(target.to_string());
            continue;
        }
        let rel = path.trim_start_matches("./");
        let link_path = if rel.is_empty() { "AGENTS.md" } else { rel };
        if !repo_path_exists(link_path)
            || !resolved_repo_path_is_contained(&d2b_contract_tests::repo_root(), link_path)
        {
            missing.push(target.to_string());
            continue;
        }
        if !anchor.is_empty() {
            let target_path = d2b_contract_tests::repo_root().join(link_path);
            let content = if link_path == "AGENTS.md" {
                agents.clone()
            } else if target_path.is_file() {
                read_repo_file(link_path)
            } else if target_path.is_dir() {
                let index = target_path.join("README.md");
                index
                    .is_file()
                    .then(|| {
                        index
                            .strip_prefix(d2b_contract_tests::repo_root())
                            .expect("Markdown directory index stays in repo")
                            .to_string_lossy()
                            .into_owned()
                    })
                    .filter(|index| {
                        resolved_repo_path_is_contained(&d2b_contract_tests::repo_root(), index)
                    })
                    .map(|index| read_repo_file(&index))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            if content.is_empty() || !markdown_has_anchor(&content, anchor) {
                missing.push(target.to_string());
            }
        }
    }
    assert!(
        missing.is_empty(),
        "AGENTS.md links to {} missing path(s) or anchor(s): {}",
        missing.len(),
        missing.join(", ")
    );
}

#[test]
fn agents_md_defines_the_single_authority_and_tiered_routes() {
    let agents = read_repo_file("AGENTS.md");
    for required in [
        "single operational authority",
        "Every code change uses Compound Engineering",
        "Clear bounded change",
        "Open-ended bug",
        "Larger or product-ambiguous work",
        "ce-work",
        "ce-debug",
        "ce-brainstorm",
        "ce-plan",
        "Ponytail",
        "Caveman",
        "transient communication only",
    ] {
        assert!(
            agents.contains(required),
            "AGENTS.md is missing contributor workflow anchor: {required}"
        );
    }
}

#[test]
fn agents_md_defines_model_preferences_and_exact_ce_profile() {
    let agents = read_repo_file("AGENTS.md");
    for required in [
        "gpt-5.6-sol",
        "xhigh reasoning and long context",
        "gpt-5.6-luna",
        "strongest native",
        "record that substitution only in the transient handoff",
        "\nce-work\nce-work mode:return-to-caller <plan-path>\n",
        "ce-code-review mode:agent",
        "ce-commit-push-pr branding:off babysit:off",
        "ce-babysit-pr posture:target",
    ] {
        assert!(
            agents.contains(required),
            "AGENTS.md is missing model/profile anchor: {required}"
        );
    }
}

#[test]
fn agents_md_defines_reviewed_head_and_guarded_merge_contract() {
    let agents = read_repo_file("AGENTS.md");
    for required in [
        "independent review",
        "fresh review",
        "Missing review evidence fails closed",
        "ce-babysit-pr",
        "expected-head guard",
        "normal squash",
        "Never use admin, auto-merge, bypass",
        "observed base",
        "nix/gas-city-contributor/**",
        "managed authority unchanged",
    ] {
        assert!(
            agents.contains(required),
            "AGENTS.md is missing reviewed-head or boundary anchor: {required}"
        );
    }
}

#[test]
fn strategy_is_concise_product_direction_without_operational_policy() {
    const BUDGET: usize = 4_000;
    let strategy = read_repo_file("STRATEGY.md");
    assert!(
        strategy.len() <= BUDGET,
        "STRATEGY.md is {} bytes, over its {BUDGET}-byte product-direction budget",
        strategy.len()
    );
    for required in [
        "Product purpose",
        "Target user and outcome",
        "daemon-only control plane",
        "Isolation and security posture",
        "Declarative contract",
        "Current direction",
        "d2bd",
        "d2b-priv-broker",
        "microVM",
    ] {
        assert!(
            strategy.contains(required),
            "STRATEGY.md is missing product anchor: {required}"
        );
    }
    for forbidden in [
        "Ponytail",
        "Caveman",
        "skill",
        "model",
        "ce-work",
        "gpt-5.6",
        "pull request",
        "expected-head",
        "ce-code-review",
    ] {
        assert!(
            !strategy.contains(forbidden),
            "STRATEGY.md must not carry operational policy: {forbidden}"
        );
    }
}

#[test]
fn strategy_is_governed_by_the_process_marker_gate() {
    let gate = read_repo_file("tests/tools/tier0-first-pass.sh");
    assert!(
        gate.contains("README.md|SECURITY.md|STRATEGY.md|docs/reference/*"),
        "STRATEGY.md must remain in the process-marker gate's full-file shipped-prose class"
    );
}

// ---------------------------------------------------------------------------
// Migrated from tests/manpage-completeness-eval.sh.
//
// Asserts that every top-level clap subcommand declared in
// `packages/d2b/src/dispatch.rs` (`enum ModernCommand { ... }`) is documented as
// a section in the committed d2b(1) manpage at `docs/manpages/d2b.1`.
// clap_mangen emits one `.TP` entry per subcommand under the SUBCOMMANDS block
// (rendered as `d2b-<name>(1)`); a new verb that lands without rerunning
// `cargo xtask gen-cli-shell-artifacts` silently drops out of the manpage. This
// gate fails closed on that drift without needing a cargo toolchain.
// ---------------------------------------------------------------------------
#[test]
fn manpage_documents_every_top_level_subcommand() {
    let cli_rel = "packages/d2b/src/dispatch.rs";
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

const CLI_DOC_ROOTS: &[&str] = &[
    "README.md",
    "templates/default",
    "examples",
    "docs/how-to",
    "docs/reference",
    "docs/explanation",
    "tests/integration/live/live-vm-smoke.sh",
];

/// `docs/reference/cli-output/` and `docs/reference/error-codes.md` are
/// drift-governed artifact sets. Their committed compatibility prose/schemas
/// are emitted from legacy DTOs; the current typed command contract lives in
/// `docs/reference/cli-contract.md`. Do not broaden this exemption to other
/// reference docs.
fn generated_cli_compatibility_artifact(rel: &str) -> bool {
    rel.starts_with("docs/reference/cli-output/") || rel == "docs/reference/error-codes.md"
}

/// Explicitly historical how-tos retain the old command as migration input.
/// Keep this list narrow: current how-tos under the same directory remain
/// governed by the scan below.
fn historical_cli_document(rel: &str) -> bool {
    matches!(
        rel,
        "docs/how-to/migrate-d2b-v0-to-v1.md"
            | "docs/how-to/migrate-d2b-v1-0-to-v1-1.md"
            | "docs/how-to/migrate-d2b-v1-1-to-v1-2.md"
            | "docs/how-to/migrate-d2b-v1-2-to-v1-3.md"
            | "docs/how-to/migrate-d2b-v1-2-to-v2.md"
            | "docs/how-to/migrate-nixos-to-daemon.md"
            | "docs/how-to/migrate-usbip-yubikey-to-security-key.md"
    )
}

fn cli_doc_path_is_governed(rel: &str) -> bool {
    if rel.starts_with("docs/reference/schemas/") {
        return false;
    }
    if generated_cli_compatibility_artifact(rel) {
        return false;
    }
    if historical_cli_document(rel) {
        return false;
    }
    matches!(
        std::path::Path::new(rel)
            .extension()
            .and_then(|ext| ext.to_str()),
        Some("md" | "nix" | "sh")
    )
}

fn collect_cli_doc_paths(repo_root: &Path, rel: &str, paths: &mut Vec<String>) {
    let root = repo_root.join(rel);
    let metadata = std::fs::metadata(&root)
        .unwrap_or_else(|err| panic!("CLI policy path is unreadable: {rel}: {err}"));
    if metadata.is_file() {
        if cli_doc_path_is_governed(rel) {
            paths.push(rel.to_owned());
        }
        return;
    }
    let entries = std::fs::read_dir(&root)
        .unwrap_or_else(|err| panic!("CLI policy directory is unreadable: {rel}: {err}"));
    for entry in entries {
        let entry = entry
            .unwrap_or_else(|err| panic!("CLI policy directory entry is unreadable: {rel}: {err}"));
        let child = entry.path();
        let child_rel = child
            .strip_prefix(repo_root)
            .expect("CLI policy child stays under repo root")
            .to_string_lossy()
            .into_owned();
        if child.is_dir() {
            collect_cli_doc_paths(repo_root, &child_rel, paths);
        } else if cli_doc_path_is_governed(&child_rel) {
            paths.push(child_rel);
        }
    }
}

fn historical_cli_line(line: &str) -> bool {
    static MARKER: OnceLock<Regex> = OnceLock::new();
    MARKER
        .get_or_init(|| {
            Regex::new(
                r"(?i)\bhistorical\b|\blegacy\b|\bretired\b|\bremoved\b|\bdeleted\b|\bsuccessor\b|\bmigrat(?:e|ion)\b|\barchived\b|\bdeprecated\b|\bno longer\b|\bpredates\b|\bsupersedes\b",
            )
            .expect("valid CLI history marker regex")
        })
        .is_match(line)
}

/// Return the untyped `d2b list` occurrences on one line. A positional
/// ResourceType keeps the generic command valid; flags, prose, and an omitted
/// positional are the retired inventory form.
fn has_untyped_list_occurrence(line: &str) -> bool {
    let mut offset = 0;
    while let Some(relative) = line[offset..].find("d2b list") {
        let start = offset + relative;
        if start > 0
            && (line.as_bytes()[start - 1].is_ascii_alphanumeric()
                || matches!(line.as_bytes()[start - 1], b'.' | b'-' | b'_'))
        {
            offset = start + "d2b list".len();
            continue;
        }
        let after = &line[start + "d2b list".len()..];
        let token = after.split_whitespace().next().unwrap_or("");
        let token = token.trim_matches(|ch: char| matches!(ch, '`' | '\'' | '"' | ')' | ']' | ','));
        let generic_type = token == "<RESOURCE_TYPE>"
            || token
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_uppercase())
            || token.contains(".d2bus.org/");
        if !generic_type {
            return true;
        }
        offset = start + "d2b list".len();
    }
    false
}

#[test]
fn governed_cli_docs_use_typed_guest_inventory_and_lifecycle_commands() {
    let vm_re = Regex::new(r"\bd2b\s+vm\s+(?:start|stop|restart|list|status|exec)\b")
        .expect("valid retired VM command regex");
    let status_re =
        Regex::new(r"\bd2b\s+status(?:\s|`|$)").expect("valid retired status command regex");
    let mut paths = Vec::new();
    let repo_root = d2b_contract_tests::repo_root();
    for root in CLI_DOC_ROOTS {
        collect_cli_doc_paths(&repo_root, root, &mut paths);
    }
    paths.sort();
    paths.dedup();
    assert!(
        !paths.is_empty(),
        "CLI policy scan found no governed documentation or live script paths"
    );

    let mut violations = Vec::new();
    for rel in paths {
        let content = std::fs::read_to_string(repo_root.join(&rel))
            .unwrap_or_else(|err| panic!("CLI policy file is unreadable: {rel}: {err}"));
        for (line_number, line) in content.lines().enumerate() {
            if historical_cli_line(line) {
                continue;
            }
            if has_untyped_list_occurrence(line) {
                violations.push(format!(
                    "{rel}:{} uses untyped `d2b list`; use `d2b guest list` or `d2b list <RESOURCE_TYPE>`",
                    line_number + 1
                ));
            }
            if vm_re.is_match(line) {
                violations.push(format!(
                    "{rel}:{} uses a retired `d2b vm` lifecycle/exec form",
                    line_number + 1
                ));
            }
            if status_re.is_match(line) {
                violations.push(format!(
                    "{rel}:{} uses retired untyped `d2b status`",
                    line_number + 1
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "current governed docs or live scripts reintroduced retired CLI forms:\n{}",
        violations.join("\n")
    );

    let readme = read_repo_file("README.md");
    let smoke = read_repo_file("tests/integration/live/live-vm-smoke.sh");
    assert!(
        readme.contains("d2b guest list"),
        "README must document typed Guest inventory"
    );
    assert!(
        smoke.contains("d2b exec run \"Guest/$vm\""),
        "live VM smoke must use typed Guest exec ResourceRefs"
    );
    assert!(
        smoke.contains("resources.d2bus.org/v3")
            && smoke.contains(".status.phase")
            && smoke.contains(".status.conditions"),
        "live VM smoke must parse v3 Guest status and readiness fields"
    );
}

/// Faithful port of the bash gate's `awk` extraction of the `enum ModernCommand`
/// subcommand set. Two forms are recognised inside the enum block:
///   1. An explicit override `#[command(name = "...")]` on the line immediately
///      preceding a variant.
///   2. The default clap conversion: a `Ident(...)` variant whose PascalCase
///      identifier becomes kebab-case lowercase.
///
/// Only variants of the form `^<ws>Ident(` (a tuple-data variant) are detected,
/// exactly as the bash awk parser did.
fn expected_subcommands(cli_src: &str) -> BTreeSet<String> {
    let enum_start = Regex::new(
        r"^[[:space:]]*(?:pub(?:\([^)]*\))?[[:space:]]+)?enum ModernCommand[[:space:]]*\{",
    )
    .unwrap();
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
