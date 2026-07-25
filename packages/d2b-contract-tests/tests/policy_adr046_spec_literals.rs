//! ADR 0046 spec-literal drift lints: mechanical enforcement of three frozen
//! decisions whose contradicting examples a human enumeration already missed
//! (a first pass reported "roughly 40+ across seven files" and a later sweep
//! found more files). A frozen decision whose examples drift silently is worse
//! than no freeze, so each class below is enforced by a scan rather than by
//! eye.
//!
//!   * D103 - every persistent datetime is exactly `YYYY-MM-DDTHH:MM:SS.sssZ`
//!     (24 bytes, uppercase `T`/`Z`, exactly three fractional digits).
//!   * D104 - a qualified ResourceType uses the single literal `.d2bus.org.`
//!     infix; a foreign-domain qualifier such as `acme.io.Widget` is rejected.
//!   * D108 - the retry delay is the integer scalar `retryAfterMs`; the earlier
//!     `retryAfter` duration-string form is superseded.
//!
//! The scan is fixture-independent: it reads the committed `docs/specs/**`
//! tree, never `D2B_FIXTURES`. Each scanner is exercised by planted-violation
//! and clean fixtures so a regression in the scanner itself fails a test rather
//! than silently passing an empty scan.
//!
//! Legitimate rejection illustrations are exempted two ways: a line carrying
//! the explicit marker `d2b-lint-allow: <code>`, and the decision-register row
//! that *defines* a rule (a lint that fails on the decision defining it is
//! worse than no lint). Nothing else is exempt.

use std::path::{Path, PathBuf};

use d2b_contract_tests::repo_root;
use regex::Regex;

/// A single violation: the repo-relative file, 1-based line number, and the
/// offending text, formatted for a fail message an author can act on directly.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Violation {
    file: String,
    line: usize,
    text: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.text)
    }
}

/// Whether `line` is exempt from the lint `code` ("D103" / "D104" / "D108").
///
/// A line is exempt when it carries the explicit `d2b-lint-allow: <code>`
/// marker, or when it is the decision-register table row that defines the rule
/// (`| <code> |` in `ADR-046-decision-register.md`). The defining row
/// legitimately quotes the rejected form.
fn is_allowed(file: &str, line: &str, code: &str) -> bool {
    if line.contains(&format!("d2b-lint-allow: {code}")) {
        return true;
    }
    file.ends_with("ADR-046-decision-register.md")
        && line.trim_start().starts_with(&format!("| {code} |"))
}

// ---------------------------------------------------------------------------
// D103 - millisecond-precision RFC 3339.
// ---------------------------------------------------------------------------

/// The one accepted spelling: 24 ASCII bytes, uppercase `T`/`Z`, exactly three
/// fractional digits.
fn d103_accept() -> Regex {
    Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$").expect("valid D103 accept regex")
}

/// A broad RFC 3339-shaped datetime candidate: a full date + `T`/`t` +
/// hh:mm:ss, plus any fractional part and any zone designator. Matching this
/// and then requiring the whole token to equal the accept form catches a
/// lowercase `t`/`z`, a numeric offset such as `+00:00`, an absent fractional
/// part, and any fractional width other than three.
fn d103_candidate() -> Regex {
    Regex::new(r"\d{4}-\d{2}-\d{2}[Tt]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:[Zz]|[+-]\d{2}:?\d{2})?")
        .expect("valid D103 candidate regex")
}

fn scan_d103(file: &str, content: &str) -> Vec<Violation> {
    let accept = d103_accept();
    let candidate = d103_candidate();
    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if is_allowed(file, line, "D103") {
            continue;
        }
        for m in candidate.find_iter(line) {
            if !accept.is_match(m.as_str()) {
                out.push(Violation {
                    file: file.to_string(),
                    line: idx + 1,
                    text: m.as_str().to_string(),
                });
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// D104 - qualified ResourceType grammar.
// ---------------------------------------------------------------------------

/// A domain-qualified UpperCamel token whose segment immediately before the
/// type is a public TLD. Anchoring on a TLD segment distinguishes a
/// vendor-ResourceType-shaped token (`acme.io.Widget`,
/// `widgets.example.org.WidgetResource`) from a gRPC service name
/// (`d2b.audit.v3.AuditService`), a well-known message type
/// (`google.protobuf.Any`), a D-Bus interface (`org.freedesktop.Notifications`,
/// whose pre-type segment is `freedesktop`, not a TLD), and a status-condition
/// path (`status.conditions.Ready`). The captured `dom` is accepted only when
/// it is qualified under `.d2bus.org`.
fn d104_candidate() -> Regex {
    Regex::new(
        r"(?P<dom>[a-z][a-z0-9-]*(?:\.[a-z0-9-]+)*\.(?:io|org|com|net|dev|app|co|ai|xyz|cloud|internal|local))\.(?P<ty>[A-Z][A-Za-z0-9]{0,62})",
    )
    .expect("valid D104 candidate regex")
}

fn scan_d104(file: &str, content: &str) -> Vec<Violation> {
    let candidate = d104_candidate();
    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if is_allowed(file, line, "D104") {
            continue;
        }
        for caps in candidate.captures_iter(line) {
            let dom = &caps["dom"];
            if dom == "d2bus.org" || dom.ends_with(".d2bus.org") {
                continue;
            }
            out.push(Violation {
                file: file.to_string(),
                line: idx + 1,
                text: caps.get(0).expect("whole match").as_str().to_string(),
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// D108 - the frozen retry scalar.
//
// D108 froze the retry delay as the integer field `retryAfterMs` and supersedes
// the earlier `retryAfter` duration-string form. It did NOT freeze the separate
// `timeout` / `backoff` / `*Deadline` duration surfaces (those are not in
// D108's impacted-spec set and the specs retain their unit-string spellings),
// so this lint is scoped to the retry scalar D108 actually froze: a
// `retryAfter`-family key that is not the exact `retryAfterMs` / `retry_after_ms`
// spelling, or a retry key carrying a duration-with-unit value instead of an
// integer.
// ---------------------------------------------------------------------------

fn d108_key() -> Regex {
    Regex::new(r"retry[_]?[Aa]fter[A-Za-z0-9_]*").expect("valid D108 key regex")
}

/// A retry key whose value on the same line is a duration-with-unit literal
/// (`"5s"`, `500ms`) rather than an integer.
fn d108_duration_value() -> Regex {
    Regex::new(r#"retry[_]?[Aa]fter[A-Za-z0-9_]*\s*[:=]\s*"?\s*\d+\s*(?:ms|s|m|h|d)\b"#)
        .expect("valid D108 duration-value regex")
}

fn scan_d108(file: &str, content: &str) -> Vec<Violation> {
    let key = d108_key();
    let dur = d108_duration_value();
    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if is_allowed(file, line, "D108") {
            continue;
        }
        for m in key.find_iter(line) {
            let id = m.as_str();
            let accepted_key = id == "retryAfterMs" || id == "retry_after_ms";
            if !accepted_key {
                out.push(Violation {
                    file: file.to_string(),
                    line: idx + 1,
                    text: format!("superseded retry key `{id}` (use `retryAfterMs`)"),
                });
            }
        }
        if dur.is_match(line) {
            out.push(Violation {
                file: file.to_string(),
                line: idx + 1,
                text:
                    "retry delay carries a duration-with-unit value (use an integer `retryAfterMs`)"
                        .to_string(),
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Real-tree enumeration.
// ---------------------------------------------------------------------------

/// Every `*.md` under `docs/specs/**`, recursively, sorted for stable output.
fn spec_markdown_files() -> Vec<PathBuf> {
    let root = repo_root().join("docs/specs");
    let mut out = Vec::new();
    collect_markdown(&root, &mut out);
    out.sort();
    out
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("policy-lint: cannot read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        let file_type = entry.file_type().expect("file type");
        if file_type.is_dir() {
            collect_markdown(&path, out);
        } else if file_type.is_file() && path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

/// Scan the real spec tree with `scanner`, returning every violation with a
/// repo-relative file path.
fn scan_spec_tree(scanner: fn(&str, &str) -> Vec<Violation>) -> Vec<Violation> {
    let root = repo_root();
    let mut out = Vec::new();
    for path in spec_markdown_files() {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("policy-lint: cannot read {}: {err}", path.display()));
        out.extend(scanner(&rel, &content));
    }
    out
}

fn report(kind: &str, violations: &[Violation]) -> String {
    let mut msg = format!(
        "{kind}: {} spec-literal violation(s) under docs/specs/**:\n",
        violations.len()
    );
    for v in violations {
        msg.push_str("  ");
        msg.push_str(&v.to_string());
        msg.push('\n');
    }
    msg
}

// ---------------------------------------------------------------------------
// Scanner fixtures - prove each scan fails on a planted violation and passes on
// the accepted form. These never touch the real tree.
// ---------------------------------------------------------------------------

#[test]
fn d103_scanner_distinguishes_conformant_from_drifted_datetimes() {
    // Accepted: exactly .sssZ.
    assert!(scan_d103("f.md", "createdAt: 2026-07-22T00:00:00.000Z").is_empty());
    // Rejected classes, each a real drift the frozen decision forbids.
    for bad in [
        "createdAt: 2026-07-22T00:00:00Z",        // absent fractional part
        "createdAt: 2026-07-22T00:00:00.12Z",     // two fractional digits
        "createdAt: 2026-07-22T00:00:00.123456Z", // six fractional digits
        "createdAt: 2026-07-22t00:00:00.000z",    // lowercase t/z
        "createdAt: 2026-07-22T00:00:00.000+00:00", // numeric offset
    ] {
        assert!(
            !scan_d103("f.md", bad).is_empty(),
            "D103 scanner must reject {bad:?}"
        );
    }
    // The explicit marker exempts a deliberate illustration.
    assert!(
        scan_d103(
            "f.md",
            "rejected: 2026-07-22T00:00:00Z  <!-- d2b-lint-allow: D103 -->"
        )
        .is_empty()
    );
    // The decision-register row that defines D103 is exempt.
    assert!(
        scan_d103(
            "docs/specs/ADR-046-decision-register.md",
            "| D103 | ... rejected 2026-07-22T00:00:00Z ... |"
        )
        .is_empty()
    );
}

#[test]
fn d104_scanner_flags_foreign_domains_only() {
    // Rejected: a foreign-domain qualifier.
    assert!(!scan_d104("f.md", "type: acme.io.Widget").is_empty());
    assert!(!scan_d104("f.md", "type: widgets.example.org.WidgetResource").is_empty());
    // Accepted: the frozen `.d2bus.org.` infix.
    assert!(scan_d104("f.md", "type: acme.d2bus.org.Widget").is_empty());
    assert!(scan_d104("f.md", "type: display-wayland.d2bus.org.WaylandSession").is_empty());
    // Not ResourceTypes, must not be flagged.
    for ok in [
        "invokes d2b.audit.v3.AuditService/Export", // versioned gRPC service
        "forbidden: google.protobuf.Any",           // well-known message type
        "the org.freedesktop.Notifications interface", // D-Bus interface
        "status.conditions.Ready is True",          // condition path
        "a vendor.qualified.Name placeholder",      // prose placeholder
    ] {
        assert!(
            scan_d104("f.md", ok).is_empty(),
            "D104 must not flag {ok:?}"
        );
    }
    // Marker + defining row exemptions.
    assert!(scan_d104("f.md", "acme.io.Widget  <!-- d2b-lint-allow: D104 -->").is_empty());
    assert!(
        scan_d104(
            "docs/specs/ADR-046-decision-register.md",
            "| D104 | ... the parser rejects `acme.io.Widget` ... |"
        )
        .is_empty()
    );
}

#[test]
fn d108_scanner_flags_superseded_retry_shapes_only() {
    // Rejected: the superseded key spelling and a duration-string value.
    assert!(!scan_d108("f.md", "retryAfter: \"5s\"").is_empty());
    assert!(!scan_d108("f.md", "retry_after: 5").is_empty());
    assert!(!scan_d108("f.md", "retryAfterSeconds: 5").is_empty());
    assert!(!scan_d108("f.md", "retryAfterMs: \"5s\"").is_empty()); // ms key, duration value
    // Accepted: the frozen integer scalar in both casings.
    assert!(scan_d108("f.md", "retryAfterMs: 5000").is_empty());
    assert!(scan_d108("f.md", "retry_after_ms: Option<u64>").is_empty());
    assert!(scan_d108("f.md", "requeue_at = now + retry_after_ms").is_empty());
    // D108 does not govern the separate timeout/backoff/deadline surfaces.
    for ok in [
        "timeout: \"30s\"",
        "backoffBase: \"1s\"",
        "startDeadline = \"30s\";",
    ] {
        assert!(
            scan_d108("f.md", ok).is_empty(),
            "D108 must not flag {ok:?}"
        );
    }
    // Marker + defining row exemptions.
    assert!(
        scan_d108(
            "f.md",
            "retryAfter duration form  <!-- d2b-lint-allow: D108 -->"
        )
        .is_empty()
    );
    assert!(
        scan_d108(
            "docs/specs/ADR-046-decision-register.md",
            "| D108 | ... supersedes the earlier `retryAfter` duration-string form ... |"
        )
        .is_empty()
    );
}

// ---------------------------------------------------------------------------
// Real-tree scans - the actual gate.
// ---------------------------------------------------------------------------

#[test]
fn docs_specs_use_millisecond_precision_datetimes() {
    let violations = scan_spec_tree(scan_d103);
    assert!(violations.is_empty(), "{}", report("D103", &violations));
}

#[test]
fn docs_specs_qualify_resource_types_under_d2bus_org() {
    let violations = scan_spec_tree(scan_d104);
    assert!(violations.is_empty(), "{}", report("D104", &violations));
}

#[test]
fn docs_specs_use_the_frozen_retry_scalar() {
    let violations = scan_spec_tree(scan_d108);
    assert!(violations.is_empty(), "{}", report("D108", &violations));
}
