//! ADR 0046 spec-literal drift lints: mechanical enforcement of three frozen
//! decisions whose contradicting examples a human enumeration already missed
//! (a first pass reported "roughly 40+ across seven files" and a later sweep
//! found more files). A frozen decision whose examples drift silently is worse
//! than no freeze, so each class below is enforced by a scan rather than by
//! eye.
//!
//! Each lint validates *semantics*, not just shape - a lint that accepts a
//! syntactically well-formed but semantically invalid literal manufactures
//! false confidence and is worse than no lint:
//!
//!   * D103 - every persistent datetime is exactly `YYYY-MM-DDTHH:MM:SS.sssZ`
//!     (24 bytes, uppercase `T`/`Z`, exactly three fractional digits) AND names
//!     a real UTC instant: month 1 to 12, a day that exists in that month
//!     (leap years included), hour 0 to 23, minute 0 to 59, second 0 to 59 (a
//!     `:60` leap second is rejected), and a year in 0001 to 9999.
//!   * D104 - a ResourceType is either one of the frozen 19 standard names or a
//!     qualified `<provider>.d2bus.org.<Type>` token whose single provider
//!     segment matches `^[a-z][a-z0-9-]*$` at 1 to 63 bytes and whose type
//!     segment matches `^[A-Z][A-Za-z0-9]{0,62}$` at 1 to 63 bytes. A missing
//!     provider (`d2bus.org.Widget`), an extra provider segment
//!     (`foo.bar.d2bus.org.Widget`), a lowercase or overlong type, an overlong
//!     provider, and a foreign-domain qualifier (`acme.io.Widget`) are all
//!     rejected. The complete `type:` token of a resource envelope and the
//!     complete type segment of a `d2bus.org`-qualified token are extracted and
//!     passed through the SAME exact validator the Nix admission uses, so an
//!     unknown unqualified name (`type: Widget`) and a malformed qualified
//!     token (`acme.d2bus.org.1Widget`, `acme.d2bus.org.Widget_Type`) are
//!     caught rather than slipping past a looser regex-substring reject set.
//!   * D108 - the retry delay is the integer scalar `retryAfterMs`; the earlier
//!     `retryAfter` duration-string form is superseded, and a `retryAfterMs`
//!     value that is a quoted string, a floating-point literal, a
//!     duration-with-unit, a boolean/null literal, a signed integer, or a bare
//!     decimal outside the frozen `1..=86400000` range (including `0`) is
//!     rejected (the scalar is an unsigned decimal millisecond count).
//!
//! The scan is fixture-independent: it reads the committed `docs/specs/**`
//! tree, never `D2B_FIXTURES`. Each scanner is exercised by planted-violation
//! and clean fixtures so a regression in the scanner itself fails a test rather
//! than silently passing an empty scan.
//!
//! Exemptions are a pinned exact allowlist, not an author-suppressible marker.
//! A per-line `d2b-lint-allow` marker would let any future author silently
//! exempt a real violation anywhere, which is the fail-open hole this file
//! closes. The only exemption is the decision-register row that *defines* a
//! rule and legitimately quotes the form it rejects (a lint that fails on the
//! decision defining it is worse than no lint). Nothing else is exempt.

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

/// The one canonical file whose defining rows are exempt. This is an exact
/// repo-relative path, not a filename suffix: a same-named copy under any other
/// directory (`docs/specs/vendored/ADR-046-decision-register.md`, a stale
/// backup) is NOT the canonical register and earns no exemption.
const DECISION_REGISTER: &str = "docs/specs/ADR-046-decision-register.md";

/// The single decision-register row that *defines* `code`, if it exists and is
/// unique. `file` must be exactly [`DECISION_REGISTER`]; the row is the table
/// line whose first cell is exactly `code` (`| <code> | ... |`). A code that
/// appears in zero rows, or in more than one first cell, yields `None` - a
/// duplicated or absent defining row is fail-closed (nothing is exempted)
/// rather than fail-open (a prefix match that would silently exempt every line
/// beginning `| <code> |`, including a body row that merely re-quotes the code).
///
/// There is deliberately no per-line author-supplied marker: an inline
/// `d2b-lint-allow` escape hatch could suppress a real violation anywhere in
/// the tree, which is exactly the fail-open behaviour this gate exists to
/// prevent. The returned exemption is the *exact* defining line; a line that
/// merely shares its `| <code> |` prefix is not exempt.
fn defining_row<'a>(file: &str, content: &'a str, code: &str) -> Option<&'a str> {
    if file != DECISION_REGISTER {
        return None;
    }
    let mut found: Option<&str> = None;
    for line in content.lines() {
        let Some(rest) = line.trim_start().strip_prefix('|') else {
            continue;
        };
        let first_cell = rest.split('|').next().unwrap_or("").trim();
        if first_cell == code {
            if found.is_some() {
                // More than one row claims this code: ambiguous, exempt nothing.
                return None;
            }
            found = Some(line);
        }
    }
    found
}

// ---------------------------------------------------------------------------
// D103 - millisecond-precision RFC 3339, validated semantically.
// ---------------------------------------------------------------------------

/// A broad RFC 3339-shaped datetime candidate: a full date + `T`/`t` +
/// hh:mm:ss, plus any fractional part and any zone designator. Matching this
/// and then requiring the whole token to be a conformant instant catches a
/// lowercase `t`/`z`, a numeric offset such as `+00:00`, an absent fractional
/// part, any fractional width other than three, and - crucially - a
/// well-formed-but-impossible calendar value or a `:60` leap second.
fn d103_candidate() -> Regex {
    Regex::new(r"\d{4}-\d{2}-\d{2}[Tt]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:[Zz]|[+-]\d{2}:?\d{2})?")
        .expect("valid D103 candidate regex")
}

fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Whether `s` is exactly the frozen 24-byte `YYYY-MM-DDTHH:MM:SS.sssZ` form
/// AND names a real UTC instant. Both the byte shape and every field range are
/// checked, so `2026-13-45T25:61:61.000Z` (impossible calendar) and
/// `2026-07-22T23:59:60.000Z` (leap second) are rejected even though they match
/// the accept *shape*.
fn d103_is_conformant(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 24 {
        return false;
    }
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
        || bytes[23] != b'Z'
    {
        return false;
    }
    for &idx in &[0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18, 20, 21, 22] {
        if !bytes[idx].is_ascii_digit() {
            return false;
        }
    }
    let field =
        |lo: usize, hi: usize| -> u32 { s[lo..hi].parse().expect("digits validated above") };
    let year = field(0, 4);
    let month = field(5, 7);
    let day = field(8, 10);
    let hour = field(11, 13);
    let minute = field(14, 16);
    let second = field(17, 19);
    (1..=9999).contains(&year)
        && (1..=12).contains(&month)
        && (1..=days_in_month(year, month)).contains(&day)
        && hour <= 23
        && minute <= 59
        && second <= 59
}

/// A persistent-timestamp authoring context: a field whose key ends in `At`
/// (`createdAt`, `lastReconciledAt`, `lastTransitionAt`, ...) carrying a scalar
/// value, in YAML (`key: value`), Nix (`key = value;`), or JSON
/// (`"key": value`). The value is captured verbatim so it can be validated
/// exactly, catching a malformed instant the broad [`d103_candidate`] shape
/// never matches (a date with no time, a `HH:MM` with no seconds, a
/// single-digit month). A key ending in `Ms`/`UnixMs` (`expiresAtUnixMs`) is
/// NOT an `At` field and is left alone.
fn d103_at_field() -> Regex {
    Regex::new(r#"(?P<key>[A-Za-z][A-Za-z0-9_]*At)\s*[:=]\s*(?P<val>"[^"]*"|'[^']*'|[^\s,;)}]+)"#)
        .expect("valid D103 at-field regex")
}

/// Whether `val` is attempting to be a calendar datetime: it begins with a
/// four-digit year followed by `-`. This keeps the at-field pass from judging a
/// prose expression (`now()`), an identifier, a bare unix-ms integer, or a
/// `<placeholder>` as a malformed instant - only a value that is trying to be a
/// `YYYY-...` datetime and failing is a real drift.
fn d103_looks_like_date(val: &str) -> bool {
    let bytes = val.as_bytes();
    bytes.len() >= 5 && bytes[..4].iter().all(u8::is_ascii_digit) && bytes[4] == b'-'
}

fn scan_d103(file: &str, content: &str) -> Vec<Violation> {
    let candidate = d103_candidate();
    let at_field = d103_at_field();
    let exempt = defining_row(file, content, "D103");
    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if Some(line) == exempt {
            continue;
        }
        // Shape-first pass: any RFC 3339-shaped candidate anywhere on the line
        // that is not the exact conformant instant.
        for m in candidate.find_iter(line) {
            if !d103_is_conformant(m.as_str()) {
                out.push(Violation {
                    file: file.to_string(),
                    line: idx + 1,
                    text: m.as_str().to_string(),
                });
            }
        }
        // Authoring-context pass: a timestamp field whose value is neither a
        // placeholder nor the exact conformant instant, including malformed
        // values the shape pass never matched.
        for caps in at_field.captures_iter(line) {
            let raw = &caps["val"];
            let val = raw
                .trim_matches('"')
                .trim_matches('\'')
                .split('#')
                .next()
                .unwrap_or("")
                .trim();
            // Only judge a value that is trying to be a `YYYY-...` datetime; a
            // placeholder, an identifier, a bare integer, or a prose expression
            // is not a malformed instant.
            if !d103_looks_like_date(val) {
                continue;
            }
            if d103_is_conformant(val) {
                continue;
            }
            // A conformant candidate is already reported by the shape pass; a
            // non-conformant value the shape pass matched is likewise already
            // reported. Only add the value here when the shape pass could not
            // have seen it - i.e. it carries no candidate at all.
            if candidate.find(val).is_none() {
                let key = &caps["key"];
                out.push(Violation {
                    file: file.to_string(),
                    line: idx + 1,
                    text: format!(
                        "`{key}` value `{val}` is not the frozen `YYYY-MM-DDTHH:MM:SS.sssZ` instant"
                    ),
                });
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// D104 - ResourceType grammar, validated exactly.
//
// A ResourceType is either one of the frozen 19 standard names (unqualified) or
// a qualified `<provider>.d2bus.org.<Type>` token. The qualified grammar (D080
// / D104) is exact: a single `<provider>` segment matching the resource-name
// grammar `^[a-z][a-z0-9-]*$` at 1 to 63 bytes, the single literal `.d2bus.org.`
// separator, and a `<Type>` segment matching `^[A-Z][A-Za-z0-9]{0,62}$` at 1 to
// 63 bytes. The lint scans for two candidate shapes and validates each fully:
// any token qualified under `d2bus.org`, and any foreign-domain illustration
// (`acme.io.Widget`) that D080 makes inadmissible.
// ---------------------------------------------------------------------------

/// The frozen 19-type standard catalog (D035): the only admissible *unqualified*
/// ResourceType names.
const STANDARD_TYPES: [&str; 19] = [
    "Zone",
    "ZoneLink",
    "Provider",
    "Host",
    "Guest",
    "Process",
    "EphemeralProcess",
    "Network",
    "Volume",
    "Credential",
    "Device",
    "Endpoint",
    "User",
    "Role",
    "RoleBinding",
    "Quota",
    "EmergencyPolicy",
    "ResourceExport",
    "ResourceImport",
];

/// Whether `provider` matches the resource-name grammar `^[a-z][a-z0-9-]*$` at
/// 1 to 63 bytes. A multi-segment value (an embedded `.`) fails here, which is
/// how an extra provider segment such as `foo.bar` is rejected.
fn is_valid_provider_segment(provider: &str) -> bool {
    let bytes = provider.as_bytes();
    (1..=63).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Whether `ty` matches the D104 `<Type>` grammar `^[A-Z][A-Za-z0-9]{0,62}$` at
/// 1 to 63 bytes.
fn is_valid_type_segment(ty: &str) -> bool {
    let bytes = ty.as_bytes();
    (1..=63).contains(&bytes.len())
        && bytes[0].is_ascii_uppercase()
        && bytes.iter().all(|b| b.is_ascii_alphanumeric())
}

/// Whether `token` is an admissible ResourceType: either one of the 19 standard
/// unqualified names, or an exact `<provider>.d2bus.org.<Type>` qualification.
fn is_valid_resource_type(token: &str) -> bool {
    if STANDARD_TYPES.contains(&token) {
        return true;
    }
    // Qualified form: split off the trailing `.<Type>`, require the remainder to
    // be exactly `<provider>.d2bus.org`, and validate both segments.
    let Some((rest, ty)) = token.rsplit_once('.') else {
        return false;
    };
    let Some(provider) = rest.strip_suffix(".d2bus.org") else {
        return false;
    };
    is_valid_provider_segment(provider) && is_valid_type_segment(ty)
}

/// Any token qualified under `d2bus.org`: `<provider-path>?.d2bus.org.<ty>`,
/// with the provider path and type captured for exact validation. The `\b`
/// anchor keeps a glued non-token like `foobard2bus.org` from matching with an
/// empty provider path. The type segment is captured with the *permissive*
/// grammar `[A-Za-z0-9_][A-Za-z0-9_-]*` - broader than the accepted
/// `[A-Z][A-Za-z0-9]*` - so a malformed type such as `1Widget` (leading digit)
/// or `Widget_Type` (underscore) is captured whole and then rejected by the
/// exact validator, instead of the accept-shape stopping short and silently
/// passing the malformation.
fn d104_d2bus_candidate() -> Regex {
    Regex::new(
        r"\b(?P<qual>(?:[a-z0-9][a-z0-9.-]*\.)?d2bus\.org)\.(?P<ty>[A-Za-z0-9_][A-Za-z0-9_-]*)",
    )
    .expect("valid D104 d2bus candidate regex")
}

/// A top-level (zero-indent) `type:` field in a YAML resource envelope, or the
/// quoted `type` value in a Nix/JSON resource declaration. The captured value
/// is the *complete* ResourceType token so it can pass through the exact
/// validator; a bare unknown name (`type: Widget`) that no qualified/foreign
/// substring scan would ever see is caught here. This context only fires inside
/// a code block that is an actual resource envelope (carries a top-level
/// `apiVersion:`), so a component-descriptor `type: controller`, a
/// deployment-service `type: service`, and a condition-fragment `type: Ready`
/// - none of which are ResourceType declarations - are not misread as one.
fn d104_type_field() -> Regex {
    Regex::new(r#"^type\s*[:=]\s*"?(?P<val>[A-Za-z0-9][A-Za-z0-9._-]*)"?;?\s*(?:#.*)?$"#)
        .expect("valid D104 type-field regex")
}

/// Whether `block` (the body lines of one fenced code block) is a resource
/// envelope: it carries a top-level `apiVersion:` key. Only then is a
/// zero-indent `type:` inside it a ResourceType authoring context.
fn block_is_envelope(block: &[&str]) -> bool {
    block
        .iter()
        .any(|line| line.starts_with("apiVersion:") || line.starts_with("\"apiVersion\""))
}

/// Iterate fenced code blocks, invoking `visit(start_line_index, body_lines)`
/// for each. `start_line_index` is the 0-based index of the block's first body
/// line in `content`. Fences are ```` ``` ```` optionally followed by a
/// language tag; the closing fence is any bare ```` ``` ````.
fn for_each_code_block<'a>(content: &'a str, mut visit: impl FnMut(usize, &[&'a str])) {
    let lines: Vec<&str> = content.lines().collect();
    let mut idx = 0;
    while idx < lines.len() {
        let trimmed = lines[idx].trim_start();
        if trimmed.starts_with("```") {
            let body_start = idx + 1;
            let mut end = body_start;
            while end < lines.len() && !lines[end].trim_start().starts_with("```") {
                end += 1;
            }
            visit(body_start, &lines[body_start..end.min(lines.len())]);
            idx = end + 1;
        } else {
            idx += 1;
        }
    }
}

fn scan_d104(file: &str, content: &str) -> Vec<Violation> {
    let d2bus = d104_d2bus_candidate();
    let foreign = d104_foreign_candidate();
    let type_field = d104_type_field();
    let exempt = defining_row(file, content, "D104");
    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if Some(line) == exempt {
            continue;
        }

        // Tokens qualified under d2bus.org: validate the exact grammar.
        for caps in d2bus.captures_iter(line) {
            let qual = &caps["qual"];
            let ty = &caps["ty"];
            let provider = qual.strip_suffix(".d2bus.org");
            let valid = matches!(provider, Some(p) if is_valid_provider_segment(p))
                && is_valid_type_segment(ty);
            if !valid {
                out.push(Violation {
                    file: file.to_string(),
                    line: idx + 1,
                    text: caps.get(0).expect("whole match").as_str().to_string(),
                });
            }
        }

        // Foreign-domain illustrations: inadmissible under D080. Tokens that are
        // in fact qualified under d2bus.org are validated above, not here.
        for caps in foreign.captures_iter(line) {
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

    // Authoring-context pass: the complete `type:` token of every resource
    // envelope, passed through the exact validator so an unknown unqualified
    // name is caught where no substring scan would see it.
    for_each_code_block(content, |start, block| {
        if !block_is_envelope(block) {
            return;
        }
        for (offset, line) in block.iter().enumerate() {
            let Some(caps) = type_field.captures(line) else {
                continue;
            };
            let val = &caps["val"];
            if !is_valid_resource_type(val) {
                let line_no = start + offset + 1;
                if Some(*line) != exempt {
                    out.push(Violation {
                        file: file.to_string(),
                        line: line_no,
                        text: format!("ResourceType `{val}` is neither a standard type nor a valid `<provider>.d2bus.org.<Type>` qualification"),
                    });
                }
            }
        }
    });

    out.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.text.cmp(&b.text)));
    out.dedup();
    out
}

/// A domain-qualified UpperCamel token whose segment immediately before the
/// type is a public TLD. Anchoring on a TLD segment distinguishes a
/// vendor-ResourceType-shaped token (`acme.io.Widget`,
/// `widgets.example.org.WidgetResource`) from a gRPC service name
/// (`d2b.audit.v3.AuditService`), a well-known message type
/// (`google.protobuf.Any`), a D-Bus interface (`org.freedesktop.Notifications`,
/// whose pre-type segment is `freedesktop`, not a TLD), and a status-condition
/// path (`status.conditions.Ready`). Tokens qualified under `d2bus.org` are
/// handled by [`d104_d2bus_candidate`] instead and skipped here.
fn d104_foreign_candidate() -> Regex {
    Regex::new(
        r"\b(?P<dom>[a-z][a-z0-9-]*(?:\.[a-z0-9-]+)*\.(?:io|org|com|net|dev|app|co|ai|xyz|cloud|internal|local))\.(?P<ty>[A-Z][A-Za-z0-9]*)",
    )
    .expect("valid D104 foreign candidate regex")
}

// ---------------------------------------------------------------------------
// D108 - the frozen retry scalar.
//
// D108 froze the retry delay as the integer field `retryAfterMs` and supersedes
// the earlier `retryAfter` duration-string form. It did NOT freeze the separate
// `timeout` / `backoff` / `*Deadline` duration surfaces (those are not in
// D108's impacted-spec set and the specs retain their unit-string spellings),
// so this lint is scoped to the retry scalar D108 actually froze. It rejects:
//
//   * a `retryAfter`-family key that is not the exact `retryAfterMs` /
//     `retry_after_ms` spelling (a superseded key);
//   * a `retryAfterMs` value that is a duration-with-unit (`"5s"`, `500ms`), a
//     quoted string (`"5000"`), or a floating-point literal (`5.5`);
//   * a `retryAfterMs` value that is a boolean/null literal (`true`, `null`) or
//     a signed integer (`-1`); the scalar is an unsigned decimal count;
//   * a bare decimal outside the frozen `1..=86400000` range, including `0`
//     (absence is spelled by omitting the field, not by `0`).
//
// A retry key used as a type annotation (`retry_after_ms: Option<u64>`) or in
// an expression (`x = now + retry_after_ms`) is NOT a value assignment and is
// left alone; those forms appear verbatim in the Accepted specs.
// ---------------------------------------------------------------------------

fn d108_key() -> Regex {
    Regex::new(r"retry[_]?[Aa]fter[A-Za-z0-9_]*").expect("valid D108 key regex")
}

/// A retry-key value assignment: `retryAfter...: <value>` or `= <value>`,
/// capturing the value token (a quoted string, or a run up to the next
/// delimiter) so it can be classified.
fn d108_assignment() -> Regex {
    Regex::new(
        r#"(?P<key>retry[_]?[Aa]fter[A-Za-z0-9_]*)\s*[:=]\s*(?P<val>"[^"]*"|'[^']*'|[^\s,;)}]+)"#,
    )
    .expect("valid D108 assignment regex")
}

/// A floating-point literal: one or more digits, a dot, one or more digits.
fn d108_float_value() -> Regex {
    Regex::new(r"^\d+\.\d+$").expect("valid D108 float regex")
}

/// A duration-with-unit literal: digits immediately followed by a time unit.
fn d108_duration_value() -> Regex {
    Regex::new(r"^\d+\s*(?:ms|s|m|h|d)$").expect("valid D108 duration regex")
}

/// A bare unsigned decimal integer (no sign, no radix prefix, no separators).
fn d108_bare_decimal() -> Regex {
    Regex::new(r"^\d+$").expect("valid D108 bare-decimal regex")
}

/// A signed decimal integer: a leading `+`/`-` on an otherwise-decimal literal.
/// `retryAfterMs` is unsigned, so a signed literal is rejected outright.
fn d108_signed_decimal() -> Regex {
    Regex::new(r"^[+-]\d+$").expect("valid D108 signed-decimal regex")
}

/// The frozen inclusive `retryAfterMs` range (D108): 1 ms to 24 h. `0` is
/// rejected so absence has exactly one spelling; `86400001` and up exceed the
/// EphemeralProcess failed-TTL ceiling.
const D108_MIN_MS: u64 = 1;
const D108_MAX_MS: u64 = 86_400_000;

/// Classify a retry-key value: `Some(reason)` when it is a literal D108 forbids
/// (a non-integer literal, or an out-of-range/zero/signed integer), `None` when
/// it is a valid bare decimal in range OR a non-value form (a type annotation,
/// an identifier, an expression fragment, a `<placeholder>`) the lint must not
/// flag.
fn d108_value_reason(val: &str) -> Option<String> {
    if val.starts_with('"') || val.starts_with('\'') {
        return Some("a quoted string".to_string());
    }
    if d108_float_value().is_match(val) {
        return Some("a floating-point literal".to_string());
    }
    if d108_duration_value().is_match(val) {
        return Some("a duration-with-unit".to_string());
    }
    if matches!(val, "true" | "false" | "null") {
        return Some(format!("the non-integer literal `{val}`"));
    }
    if d108_signed_decimal().is_match(val) {
        return Some(format!("the signed integer `{val}`"));
    }
    if d108_bare_decimal().is_match(val) {
        // A bare decimal is the only value shape D108 admits; enforce its range.
        let parsed = val.parse::<u64>().unwrap_or(u64::MAX);
        if parsed == 0 {
            return Some("zero (absence is spelled by omitting the field, not by 0)".to_string());
        }
        if !(D108_MIN_MS..=D108_MAX_MS).contains(&parsed) {
            return Some(format!(
                "out of the frozen {D108_MIN_MS}..={D108_MAX_MS} millisecond range"
            ));
        }
        return None;
    }
    // Not a literal we can judge: an identifier, a type annotation, an
    // expression fragment, or a `<placeholder>`. Leave it alone.
    None
}

fn scan_d108(file: &str, content: &str) -> Vec<Violation> {
    let key = d108_key();
    let assignment = d108_assignment();
    let exempt = defining_row(file, content, "D108");
    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if Some(line) == exempt {
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
        for caps in assignment.captures_iter(line) {
            let val = &caps["val"];
            if let Some(reason) = d108_value_reason(val) {
                let key = &caps["key"];
                out.push(Violation {
                    file: file.to_string(),
                    line: idx + 1,
                    text: format!(
                        "`{key}` value `{val}` is {reason} (use a bare decimal millisecond count)"
                    ),
                });
            }
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
    // Accepted: exactly .sssZ and a real calendar instant.
    assert!(scan_d103("f.md", "createdAt: 2026-07-22T00:00:00.000Z").is_empty());
    assert!(scan_d103("f.md", "epoch: 1970-01-01T00:00:00.000Z").is_empty());
    // A leap day in a leap year is a real instant.
    assert!(scan_d103("f.md", "leap: 2024-02-29T12:00:00.000Z").is_empty());
    // Rejected shape classes, each a real drift the frozen decision forbids.
    for bad in [
        "createdAt: 2026-07-22T00:00:00Z",        // absent fractional part
        "createdAt: 2026-07-22T00:00:00.12Z",     // two fractional digits
        "createdAt: 2026-07-22T00:00:00.123456Z", // six fractional digits
        "createdAt: 2026-07-22t00:00:00.000z",    // lowercase t/z
        "createdAt: 2026-07-22T00:00:00.000+00:00", // numeric offset
    ] {
        assert!(
            !scan_d103("f.md", bad).is_empty(),
            "D103 scanner must reject the shape {bad:?}"
        );
    }
    // Rejected semantic classes: well-formed shape, impossible instant.
    for bad in [
        "createdAt: 2026-13-45T25:61:61.000Z", // impossible month/day/time
        "createdAt: 2026-07-22T23:59:60.000Z", // leap second
        "createdAt: 2026-00-10T00:00:00.000Z", // month 0
        "createdAt: 2026-02-29T00:00:00.000Z", // Feb 29 in a non-leap year
        "createdAt: 2026-04-31T00:00:00.000Z", // April has 30 days
        "createdAt: 0000-01-01T00:00:00.000Z", // year 0
    ] {
        assert!(
            !scan_d103("f.md", bad).is_empty(),
            "D103 scanner must reject the semantically invalid {bad:?}"
        );
    }
    // The closed escape hatch: an inline `d2b-lint-allow` marker does NOT
    // exempt a drifted datetime. Only the decision-register defining row is
    // exempt.
    assert!(
        !scan_d103(
            "f.md",
            "rejected: 2026-07-22T00:00:00Z  <!-- d2b-lint-allow: D103 -->"
        )
        .is_empty(),
        "an inline allow marker must not exempt a drifted datetime"
    );
    // The decision-register row that defines D103 is exempt.
    assert!(
        scan_d103(
            "docs/specs/ADR-046-decision-register.md",
            "| D103 | ... rejected 2026-07-22T00:00:00Z ... |"
        )
        .is_empty()
    );
    // The defining-row exemption is pinned to that one file: the same row text
    // elsewhere is NOT exempt.
    assert!(
        !scan_d103(
            "docs/specs/ADR-046-resource-object-model.md",
            "| D103 | ... rejected 2026-07-22T00:00:00Z ... |"
        )
        .is_empty(),
        "the defining-row exemption must not apply outside the decision register"
    );
}

#[test]
fn d103_at_field_context_catches_malformed_instants_outside_the_candidate_shape() {
    // Malformed values the broad candidate shape never matches, in a real
    // timestamp-authoring field, must still be caught.
    for bad in [
        "createdAt: 2026-07-22",                 // date only, no time
        "lastReconciledAt: 2026-07-22T00:00Z",   // HH:MM, no seconds
        "startedAt: \"2026-7-2T00:00:00.000Z\"", // single-digit month/day
        "updatedAt: 2026-07-22 00:00:00.000Z",   // space instead of T
    ] {
        assert!(
            !scan_d103("f.md", bad).is_empty(),
            "D103 at-field context must reject {bad:?}"
        );
    }
    // Accepted at-field values: the exact instant (bare or quoted), an explicit
    // null, and a `<...>` schema placeholder.
    for ok in [
        "createdAt: 2026-07-22T00:00:00.000Z",
        "lastReconciledAt: \"2026-07-22T00:00:01.000Z\"",
        "completedAt: null",
        "lastTransitionAt: <deletionRequestedAt>",
    ] {
        assert!(
            scan_d103("f.md", ok).is_empty(),
            "D103 at-field context must accept {ok:?}"
        );
    }
    // A key ending in `Ms`, not `At`, is a unix-ms count, not an instant.
    assert!(scan_d103("f.md", "expiresAtUnixMs: 1753228801000").is_empty());
}

#[test]
fn defining_row_exemption_is_bound_to_the_exact_unique_row() {
    // A body line that merely shares the `| D103 |` prefix but is NOT the
    // unique defining row (here duplicated) is not exempt: a duplicated code
    // fails closed, exempting nothing.
    let dup = concat!(
        "| D103 | rejects 2026-07-22T00:00:00Z |\n",
        "| D103 | duplicate row 2026-07-22T00:00:00Z |\n",
    );
    assert!(
        !scan_d103("docs/specs/ADR-046-decision-register.md", dup).is_empty(),
        "a duplicated defining row must exempt nothing (fail closed)"
    );
    // Only the exact defining line is exempt; another row on the same register
    // that quotes the drifted form is still flagged.
    let two = concat!(
        "| D103 | defines the rule; rejects 2026-07-22T00:00:00Z |\n",
        "| D091 | unrelated row that also cites 2026-07-22T00:00:00Z |\n",
    );
    let violations = scan_d103("docs/specs/ADR-046-decision-register.md", two);
    assert_eq!(
        violations.len(),
        1,
        "only the exact D103 defining row is exempt: {violations:?}"
    );
    assert_eq!(violations[0].line, 2, "the unrelated D091 row is flagged");
}

#[test]
fn d104_grammar_accepts_standard_and_qualified_types_only() {
    // Every one of the 19 unqualified standard names is admissible.
    for name in STANDARD_TYPES {
        assert!(
            is_valid_resource_type(name),
            "standard type {name:?} must be admissible"
        );
    }
    // Qualified forms named by Accepted specs are admissible.
    for ok in [
        "acme.d2bus.org.Widget",
        "display-wayland.d2bus.org.WaylandSession",
        "security-key.d2bus.org.SecurityKeyBinding",
    ] {
        assert!(is_valid_resource_type(ok), "{ok:?} must be admissible");
    }
    // Inadmissible: an unknown unqualified name, a missing/extra provider
    // segment, a foreign domain, a lowercase or overlong type.
    for bad in [
        "Widget",                   // unqualified, not in the catalog
        "d2bus.org.Widget",         // missing provider segment
        "foo.bar.d2bus.org.Widget", // extra provider segment
        "acme.io.Widget",           // foreign domain
        "acme.d2bus.org.widget",    // lowercase type
        "ACME.d2bus.org.Widget",    // provider not lowercase
    ] {
        assert!(!is_valid_resource_type(bad), "{bad:?} must be inadmissible");
    }
}

#[test]
fn d104_scanner_flags_every_grammar_violation() {
    // Accepted: the frozen `.d2bus.org.` qualification.
    assert!(scan_d104("f.md", "type: acme.d2bus.org.Widget").is_empty());
    assert!(scan_d104("f.md", "type: display-wayland.d2bus.org.WaylandSession").is_empty());
    // A 63-byte type and a 63-byte provider are at the byte bound and accepted.
    let ty63: String = std::iter::once('A')
        .chain(std::iter::repeat_n('a', 62))
        .collect();
    assert!(
        scan_d104("f.md", &format!("type: acme.d2bus.org.{ty63}")).is_empty(),
        "a 63-byte type is at the bound"
    );
    let prov63: String = std::iter::once('a')
        .chain(std::iter::repeat_n('a', 62))
        .collect();
    assert!(
        scan_d104("f.md", &format!("type: {prov63}.d2bus.org.Widget")).is_empty(),
        "a 63-byte provider is at the bound"
    );

    // Rejected classes, each a real drift the frozen grammar forbids.
    let ty64: String = std::iter::once('A')
        .chain(std::iter::repeat_n('a', 63))
        .collect();
    let prov64: String = std::iter::once('a')
        .chain(std::iter::repeat_n('a', 63))
        .collect();
    let overlong_type = format!("type: acme.d2bus.org.{ty64}");
    let overlong_provider = format!("type: {prov64}.d2bus.org.Widget");
    for bad in [
        "type: acme.io.Widget",                     // foreign domain
        "type: widgets.example.org.WidgetResource", // foreign domain
        "type: d2bus.org.Widget",                   // missing provider segment
        "type: foo.bar.d2bus.org.Widget",           // extra provider segment
        "type: acme.d2bus.org.widget",              // lowercase type
        overlong_type.as_str(),                     // 64-byte type
        overlong_provider.as_str(),                 // 64-byte provider
    ] {
        assert!(
            !scan_d104("f.md", bad).is_empty(),
            "D104 scanner must reject {bad:?}"
        );
    }

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

    // The closed escape hatch: an inline marker does NOT exempt a violation.
    assert!(
        !scan_d104("f.md", "acme.io.Widget  <!-- d2b-lint-allow: D104 -->").is_empty(),
        "an inline allow marker must not exempt a foreign-domain type"
    );
    // The decision-register row that defines D104 is exempt.
    assert!(
        scan_d104(
            "docs/specs/ADR-046-decision-register.md",
            "| D104 | ... the parser rejects `acme.io.Widget` ... |"
        )
        .is_empty()
    );
    assert!(
        !scan_d104(
            "docs/specs/ADR-046-terminology-and-identities.md",
            "| D104 | ... the parser rejects `acme.io.Widget` ... |"
        )
        .is_empty(),
        "the defining-row exemption must not apply outside the decision register"
    );
}

#[test]
fn d104_catches_malformed_qualified_types_the_substring_reject_set_missed() {
    // A malformed type segment must be captured WHOLE and rejected by the exact
    // validator, not stopped short by an accept-shape that silently passes it.
    for bad in [
        "type: acme.d2bus.org.1Widget",     // leading digit
        "type: acme.d2bus.org.Widget_Type", // underscore
        "type: acme.d2bus.org.widget",      // lowercase
        "type: acme.d2bus.org.Widget-Kind", // hyphen
    ] {
        assert!(
            !scan_d104("f.md", bad).is_empty(),
            "D104 must reject the malformed qualified token {bad:?}"
        );
    }
    // The valid qualified token is still accepted.
    assert!(scan_d104("f.md", "type: acme.d2bus.org.Widget").is_empty());
}

#[test]
fn d104_type_field_context_catches_unknown_unqualified_names() {
    // An unknown unqualified `type:` inside a resource envelope is a ResourceType
    // authoring context and is rejected.
    let bad_envelope = concat!(
        "```yaml\n",
        "apiVersion: resources.d2bus.org/v3\n",
        "type: Widget\n",
        "metadata:\n",
        "  name: w\n",
        "```\n",
    );
    assert!(
        !scan_d104("f.md", bad_envelope).is_empty(),
        "an unknown unqualified type in an envelope must be flagged"
    );
    // A standard type in the same envelope shape is accepted.
    let good_envelope = concat!(
        "```yaml\n",
        "apiVersion: resources.d2bus.org/v3\n",
        "type: Host\n",
        "metadata:\n",
        "  name: h\n",
        "```\n",
    );
    assert!(
        scan_d104("f.md", good_envelope).is_empty(),
        "a standard type in an envelope must be accepted"
    );
    // A component/service descriptor and a bare condition fragment are NOT
    // resource envelopes (no apiVersion), so their `type:` is not a ResourceType.
    let descriptor = concat!(
        "```yaml\n",
        "componentId: aca-controller\n",
        "type: controller\n",
        "resourceTypes:\n",
        "  - Guest\n",
        "```\n",
    );
    assert!(
        scan_d104("f.md", descriptor).is_empty(),
        "a component descriptor `type: controller` must not be read as a ResourceType"
    );
    let condition = concat!("```yaml\n", "type: Ready\n", "status: \"True\"\n", "```\n");
    assert!(
        scan_d104("f.md", condition).is_empty(),
        "a bare condition-fragment `type: Ready` must not be read as a ResourceType"
    );
}

#[test]
fn d108_scanner_flags_superseded_and_non_integer_retry_shapes_only() {
    // Rejected: the superseded key spelling.
    assert!(!scan_d108("f.md", "retryAfter: \"5s\"").is_empty());
    assert!(!scan_d108("f.md", "retry_after: 5").is_empty());
    assert!(!scan_d108("f.md", "retryAfterSeconds: 5").is_empty());
    // Rejected: the ms key carrying a non-integer value.
    assert!(!scan_d108("f.md", "retryAfterMs: \"5s\"").is_empty()); // quoted duration
    assert!(!scan_d108("f.md", "retryAfterMs: \"5000\"").is_empty()); // quoted integer string
    assert!(!scan_d108("f.md", "retryAfterMs: 5.5").is_empty()); // floating point
    assert!(!scan_d108("f.md", "retryAfterMs: 500ms").is_empty()); // bare duration
    // Rejected: a boolean/null literal, a signed integer, zero, and an
    // out-of-range value - none is a bare decimal in the frozen 1..=86400000.
    assert!(!scan_d108("f.md", "retryAfterMs: true").is_empty()); // boolean
    assert!(!scan_d108("f.md", "retryAfterMs: null").is_empty()); // null
    assert!(!scan_d108("f.md", "retryAfterMs: -1").is_empty()); // signed
    assert!(!scan_d108("f.md", "retryAfterMs: 0").is_empty()); // zero
    assert!(!scan_d108("f.md", "retryAfterMs: 86400001").is_empty()); // over ceiling
    // Accepted: the frozen integer scalar in both casings, at the bounds.
    assert!(scan_d108("f.md", "retryAfterMs: 5000").is_empty());
    assert!(scan_d108("f.md", "retryAfterMs: 1").is_empty());
    assert!(scan_d108("f.md", "retryAfterMs: 86400000").is_empty());
    // Accepted: a type annotation or an expression, which are not value assigns.
    assert!(scan_d108("f.md", "retry_after_ms: Option<u64>").is_empty());
    assert!(
        scan_d108(
            "f.md",
            "TargetKeyUnavailable { retry_after_ms: Option<u64> },"
        )
        .is_empty()
    );
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
    // The closed escape hatch: an inline marker does NOT exempt a violation.
    assert!(
        !scan_d108("f.md", "retryAfter: \"5s\"  <!-- d2b-lint-allow: D108 -->").is_empty(),
        "an inline allow marker must not exempt a superseded retry key"
    );
    // The decision-register row that defines D108 is exempt.
    assert!(
        scan_d108(
            "docs/specs/ADR-046-decision-register.md",
            "| D108 | ... supersedes the earlier `retryAfter` duration-string form ... |"
        )
        .is_empty()
    );
    assert!(
        !scan_d108(
            "docs/specs/ADR-046-resource-api-and-authorization.md",
            "| D108 | ... supersedes the earlier `retryAfter` duration-string form ... |"
        )
        .is_empty(),
        "the defining-row exemption must not apply outside the decision register"
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
