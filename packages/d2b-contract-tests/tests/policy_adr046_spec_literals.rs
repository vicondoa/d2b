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

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

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

/// Grow a candidate match `[start, end)` outward over adjacent ASCII
/// alphanumeric bytes (and the identifier byte `_`) so a datetime carrying
/// trailing (or leading) junk is judged as the WHOLE token, not just the
/// conformant 24-byte prefix the shape regex happened to anchor. This closes
/// `2026-07-22T00:00:00.000Zjunk` and `2026-07-22T00:00:00.000Z_junk`, whose
/// leading 24 bytes are conformant but whose full token is not. Extension stops
/// at any other byte (`.`, `:`, `+`, `-`, whitespace, quotes) so a following
/// prose sentence or a numeric offset is never swept in. The candidate regex is
/// ASCII-only, so every offset here lands on a char boundary.
fn d103_extend_token(line: &str, start: usize, end: usize) -> (usize, usize) {
    let extend = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let bytes = line.as_bytes();
    let mut s = start;
    while s > 0 && extend(bytes[s - 1]) {
        s -= 1;
    }
    let mut e = end;
    while e < bytes.len() && extend(bytes[e]) {
        e += 1;
    }
    (s, e)
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
        // that, taken as its complete alphanumeric-delimited token, is not the
        // exact conformant instant. Extending over trailing/leading alnum bytes
        // is what rejects a conformant prefix with junk glued on.
        for m in candidate.find_iter(line) {
            let (s, e) = d103_extend_token(line, m.start(), m.end());
            let token = &line[s..e];
            if !d103_is_conformant(token) {
                out.push(Violation {
                    file: file.to_string(),
                    line: idx + 1,
                    text: token.to_string(),
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
    // Structural pass: parse every tagged YAML/JSON/Nix fence with the real
    // parsers and validate the COMPLETE scalar in value position. This catches a
    // datetime whose key and value are split across lines (`"createdAt":` on one
    // line, `"2026-07-22"` on the next) - which the per-line passes above cannot
    // see - and any trailing junk fused to an otherwise-conformant instant,
    // because the parser hands back the whole scalar, not a line fragment.
    out.extend(d103_structural(file, content, &candidate));
    out
}

/// Collect every `(key, scalar)` pair in value position across a parsed
/// document tree. A sequence item inherits its enclosing key so a list under an
/// `At` field is still judged as a timestamp context.
fn walk_scalars<'a>(
    node: &'a Node,
    key: Option<&'a str>,
    out: &mut Vec<(Option<&'a str>, &'a str)>,
) {
    match node {
        Node::Scalar(s) => out.push((key, s.as_str())),
        Node::Map(entries) => {
            for e in entries {
                walk_scalars(&e.val, Some(e.key.as_str()), out);
            }
        }
        Node::Seq(items) => {
            for it in items {
                walk_scalars(it, key, out);
            }
        }
        _ => {}
    }
}

/// Whether a key names a persistent-timestamp field (ends in `At`) rather than a
/// unix-ms count (`expiresAtUnixMs`) or an unrelated field.
fn d103_is_at_key(key: &str) -> bool {
    key.ends_with("At") && !key.ends_with("Ms") && !key.ends_with("UnixMs")
}

/// Structural D103 pass over tagged YAML/JSON/Nix fences. A block that carries a
/// timestamp-authoring trigger but that the real parser cannot model fails
/// closed: the drift is reported, never skipped.
fn d103_structural(file: &str, content: &str, candidate: &Regex) -> Vec<Violation> {
    let mut out = Vec::new();
    for block in fenced_blocks(content) {
        if !matches!(block.lang.as_str(), "yaml" | "yml" | "json" | "nix") {
            continue;
        }
        let mentions_at = block.lines.iter().any(|l| {
            let t = l.trim_start().trim_start_matches('"');
            t.split([':', '=']).next().is_some_and(|k| {
                let k = k.trim().trim_matches('"');
                d103_is_at_key(k)
            })
        });
        let docs = match parse_block_docs(&block.lang, &block.lines) {
            Ok(docs) => docs,
            Err(_) => {
                if mentions_at {
                    out.push(Violation {
                        file: file.to_string(),
                        line: block.body_start + 1,
                        text: "block carries a timestamp field the structural parser could not model; a lint must fail closed on an unparseable datetime context, not skip it (D103)".to_string(),
                    });
                }
                continue;
            }
        };
        for doc in &docs {
            let mut pairs = Vec::new();
            walk_scalars(doc, None, &mut pairs);
            for (key, s) in pairs {
                // A timestamp field's complete value must be the exact instant.
                if key.is_some_and(d103_is_at_key) && d103_looks_like_date(s) {
                    if !d103_is_conformant(s) {
                        out.push(Violation {
                            file: file.to_string(),
                            line: block.body_start + 1,
                            text: format!(
                                "`{}` value `{s}` is not the frozen `YYYY-MM-DDTHH:MM:SS.sssZ` instant",
                                key.unwrap_or("")
                            ),
                        });
                    }
                    continue;
                }
                // Any other scalar carrying an RFC 3339-shaped token, validated
                // as its complete alphanumeric-delimited token.
                for m in candidate.find_iter(s) {
                    let (a, b) = d103_extend_token(s, m.start(), m.end());
                    let token = &s[a..b];
                    if !d103_is_conformant(token) {
                        out.push(Violation {
                            file: file.to_string(),
                            line: block.body_start + 1,
                            text: token.to_string(),
                        });
                    }
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Structural document model (shared with policy_adr046_envelopes.rs via the
// `common` module). Parses fenced YAML/JSON/Nix blocks into a `Node` tree with
// real parsers (serde_json, serde_yaml_ng, rnix) so a literal is validated as
// the complete parsed scalar in value position, never a line-shape regex or a
// prefix match. Every parse returns a `Result`; a parse error on a block that
// carries a check's authoring trigger is treated as a violation (fail closed).
// ---------------------------------------------------------------------------

mod common;

use common::{
    Entry, Node, collect_maps, direct_child, fenced_blocks, mentions_key, parse_block_docs,
    rel_display,
};

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

/// Whether `map` is a ResourceType authoring context whose direct `type` /
/// `resourceType` field names a ResourceType: either a resource envelope (a
/// direct `apiVersion` child), or a resource declaration (a direct `type`
/// scalar together with a direct `spec` mapping). This is judged on the parsed
/// document, so a quoted JSON `"type"` and an indented Nix `type =` are read
/// identically to a bare YAML `type:` - closing the gap where the old
/// zero-indent `^type` regex validated only bare top-level YAML. A nested
/// component-descriptor `type: controller` (no `apiVersion`, no `spec`), a
/// deployment-service `type: service`, and a condition-fragment `type: Ready`
/// are not authoring contexts and are left alone.
fn d104_is_type_authoring_context(map: &[Entry]) -> bool {
    if direct_child(map, "apiVersion").is_some() {
        return true;
    }
    let has_type_scalar =
        matches!(direct_child(map, "type"), Some(e) if matches!(e.val, Node::Scalar(_)));
    let has_spec_map =
        matches!(direct_child(map, "spec"), Some(e) if matches!(e.val, Node::Map(_)));
    has_type_scalar && has_spec_map
}

/// Whether a captured type value is a schema `<placeholder>` rather than a
/// concrete ResourceType, e.g. `<ResourceType>` or `<name>`. Placeholders are
/// authored deliberately and are not grammar violations.
fn d104_is_placeholder(val: &str) -> bool {
    val.starts_with('<') || val.contains('<')
}

fn scan_d104(file: &str, content: &str) -> Vec<Violation> {
    let d2bus = d104_d2bus_candidate();
    let foreign = d104_foreign_candidate();
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

    // Authoring-context pass: parse every fenced block structurally and validate
    // the direct `type` / `resourceType` scalar of each ResourceType authoring
    // context (envelope or resource declaration). This reads a quoted JSON
    // `"type"` and an indented Nix `type =` identically to a bare YAML `type:`,
    // closing the gap where the old zero-indent `^type` regex validated only
    // bare top-level YAML. The COMPLETE parsed scalar is validated: an
    // over-qualified `acme.d2bus.org.Widget.Type` - which the substring pass
    // accepts because its capture stops at the first `.<Type>` - is rejected
    // here because `is_valid_resource_type` sees the whole dotted token. A
    // `<placeholder>` and a Nix interpolation/variable are authored deliberately
    // (modeled as `Placeholder` / `Opaque`, never `Scalar`) and are left alone.
    for block in fenced_blocks(content) {
        if !matches!(block.lang.as_str(), "yaml" | "yml" | "json" | "nix" | "") {
            continue;
        }
        let docs = match parse_block_docs(&block.lang, &block.lines) {
            Ok(docs) => docs,
            Err(_) => {
                // Fail closed: a block that clearly declares a live resource
                // envelope (an `apiVersion` marker) but that the real parser
                // cannot model must be reported, never silently skipped. A bare
                // `type =` without `apiVersion` is not necessarily a ResourceType
                // authoring context - it is also an artifact-catalog type tag
                // (`type = "provider"`), a condition-fragment `type: Ready`, or a
                // component descriptor - so it does not by itself force a fail
                // closed here; the substring passes still scan every line of an
                // unparseable block for qualified/foreign ResourceType tokens.
                let names_envelope = mentions_key(&block.lines, "apiVersion");
                if names_envelope {
                    out.push(Violation {
                        file: file.to_string(),
                        line: block.body_start + 1,
                        text: "block declares a resource envelope the structural parser could not model; a lint must fail closed on an unparseable ResourceType context, not skip it (D104)".to_string(),
                    });
                }
                continue;
            }
        };
        let mut maps = Vec::new();
        for doc in &docs {
            collect_maps(doc, &mut maps);
        }
        for map in maps {
            if !d104_is_type_authoring_context(map) {
                continue;
            }
            for key in ["type", "resourceType"] {
                let Some(entry) = direct_child(map, key) else {
                    continue;
                };
                let Node::Scalar(val) = &entry.val else {
                    continue;
                };
                if d104_is_placeholder(val) {
                    continue;
                }
                if !is_valid_resource_type(val) {
                    out.push(Violation {
                        file: file.to_string(),
                        line: block.body_start + entry.line + 1,
                        text: format!(
                            "ResourceType `{val}` is neither a standard type nor a valid `<provider>.d2bus.org.<Type>` qualification"
                        ),
                    });
                }
            }
        }
    }

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

/// A retry-key value that is a Rust type annotation or a schema `<placeholder>`
/// rather than a concrete value: `retry_after_ms: Option<u64>`, a bare Rust
/// integer type (`u64`), a `TypeName`, or `<ms>`. These are the only non-decimal
/// forms the lint tolerates; every other token in value position is rejected so
/// `1e3`, `banana`, and `nonsense` can no longer slip through the fall-through.
fn d108_is_type_or_placeholder(val: &str) -> bool {
    // A `<...>` schema placeholder, or a generic type (`Option<u64>`, `Vec<u8>`).
    if val.starts_with('<') || val.contains('<') {
        return true;
    }
    // A type name is upper-camel (`Duration`, `NonZeroU64`).
    if val.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        return true;
    }
    // A bare Rust primitive integer/scalar type used as a field annotation.
    matches!(
        val,
        "u8" | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
            | "bool"
            | "char"
            | "str"
    )
}

/// Classify a retry-key value: `Some(reason)` when it is a value D108 forbids
/// (a non-integer literal, an out-of-range/zero/signed integer, or any other
/// token that is not a bare decimal), `None` only when it is a valid bare
/// decimal in range OR a genuine non-value form (a type annotation or a
/// `<placeholder>`) the lint must not flag.
///
/// The fall-through is fail-closed: a token in value position that is neither a
/// valid bare decimal nor a recognised type/placeholder is rejected rather than
/// silently accepted. This closes `retryAfterMs: 1e3`, `retryAfterMs: banana`,
/// and `retryAfterMs: nonsense`, all of which the previous prefix/shape logic let
/// pass.
fn d108_value_reason(val: &str) -> Option<String> {
    if val.starts_with('"') || val.starts_with('\'') {
        return Some("a quoted string".to_string());
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
    // A non-finite float name is a value literal, not a type annotation, even
    // though `NaN`/`Infinity` are upper-camel shaped and would otherwise be
    // swallowed by the type/placeholder guard below. Reject them explicitly so
    // `retryAfterMs: NaN` is a violation, not an accepted "type annotation".
    if matches!(
        val.to_ascii_lowercase().as_str(),
        "nan" | "inf" | "+inf" | "-inf" | "infinity" | "+infinity" | "-infinity"
    ) {
        return Some(format!("the non-integer literal `{val}`"));
    }
    // A genuine non-value form (a type annotation or a `<placeholder>`) is left
    // alone; those forms appear verbatim in the Accepted specs.
    if d108_is_type_or_placeholder(val) {
        return None;
    }
    // Fail closed: anything else in value position is not a bare decimal.
    Some("not a bare decimal millisecond count".to_string())
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
    // Structural pass: parse every tagged YAML/JSON/Nix fence with the real
    // parsers and validate the COMPLETE parsed value of every `retryAfterMs` /
    // `retry_after_ms` field. This closes the per-line bypasses: a quoted JSON
    // key (`"retryAfterMs": "5s"`) that the assignment regex never matched
    // because a `"` sits between the key and the `:`, and a key/value split
    // across two lines. The value is judged as the parsed scalar, so `"5s"` and
    // `NaN` are rejected while an authored `<placeholder>` / Nix expression
    // (modeled as `Placeholder` / `Opaque`) is left alone.
    out.extend(d108_structural(file, content, &key));
    out
}

/// Collect every mapping `Entry` in the document tree, in author order.
fn walk_entries<'a>(node: &'a Node, out: &mut Vec<&'a Entry>) {
    match node {
        Node::Map(entries) => {
            for e in entries {
                out.push(e);
                walk_entries(&e.val, out);
            }
        }
        Node::Seq(items) => {
            for it in items {
                walk_entries(it, out);
            }
        }
        _ => {}
    }
}

/// Structural D108 pass over tagged YAML/JSON/Nix fences. Validates the parsed
/// value of the accepted retry key. Superseded key spellings are owned by the
/// per-line key pass (which sees them regardless of value), so this pass only
/// validates value position. A block that names a retry key but that the real
/// parser cannot model fails closed.
fn d108_structural(file: &str, content: &str, key_re: &Regex) -> Vec<Violation> {
    let mut out = Vec::new();
    for block in fenced_blocks(content) {
        if !matches!(block.lang.as_str(), "yaml" | "yml" | "json" | "nix") {
            continue;
        }
        let mentions_retry = block.lines.iter().any(|l| key_re.is_match(l));
        let docs = match parse_block_docs(&block.lang, &block.lines) {
            Ok(docs) => docs,
            Err(_) => {
                if mentions_retry {
                    out.push(Violation {
                        file: file.to_string(),
                        line: block.body_start + 1,
                        text: "block carries a retry field the structural parser could not model; a lint must fail closed on an unparseable retry context, not skip it (D108)".to_string(),
                    });
                }
                continue;
            }
        };
        for doc in &docs {
            let mut entries = Vec::new();
            walk_entries(doc, &mut entries);
            for e in entries {
                // Whole-key match of the retry family only: a key that merely
                // embeds the substring is not the retry field.
                if !matches!(key_re.find(&e.key), Some(m) if m.as_str() == e.key) {
                    continue;
                }
                if e.key != "retryAfterMs" && e.key != "retry_after_ms" {
                    // A superseded spelling; the per-line key pass owns it.
                    continue;
                }
                let reason = match &e.val {
                    Node::Scalar(s) => d108_value_reason(s),
                    Node::Null => Some("empty (no millisecond count)".to_string()),
                    Node::Map(_) | Node::Seq(_) => {
                        Some("not a bare decimal millisecond count".to_string())
                    }
                    // An authored placeholder, a Nix expression/variable, or an
                    // explicit `...` elision is not a concrete literal to judge.
                    Node::Placeholder | Node::Opaque | Node::Elision => None,
                };
                if let Some(reason) = reason {
                    out.push(Violation {
                        file: file.to_string(),
                        line: block.body_start + e.line + 1,
                        text: format!(
                            "`{}` value is {reason} (use a bare decimal millisecond count)",
                            e.key
                        ),
                    });
                }
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
        .unwrap_or_else(|err| panic!("policy-lint: cannot read {}: {err}", rel_display(dir)));
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
    let mut out = Vec::new();
    for path in spec_markdown_files() {
        let rel = rel_display(&path);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("policy-lint: cannot read {}: {err}", rel_display(&path)));
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

fn normalized_whitespace(content: &str) -> String {
    content.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug)]
struct CanonicalMeasurement {
    outcome: String,
    measurement: String,
}

#[derive(Clone, Copy)]
enum DerivedExpectation {
    CanonicalMeasurement,
    CanonicalFragment {
        source: &'static str,
        derived: &'static str,
    },
    OutcomeSummary(&'static str),
}

#[derive(Clone, Copy)]
struct DerivedMeasurementSite {
    path: &'static str,
    expectation: DerivedExpectation,
    copies: usize,
}

/// A measurement-shaped signature inventoried across every Markdown or JSON
/// document under `docs/**` plus `CHANGELOG.md`. These intentionally combine a
/// value with its denominator, unit, or canonical subject phrase instead of
/// scanning ambiguous bare numbers such as `13`, `20`, or `48`. A paraphrase
/// that drops every such signature is not mechanically identifiable and
/// remains review-only.
#[derive(Clone, Copy)]
struct MeasurementInventoryPattern {
    description: &'static str,
    regex: &'static str,
    copies: usize,
}

struct MeasurementSpec {
    name: &'static str,
    threshold: &'static str,
    expected_outcome: &'static str,
    fingerprint: &'static str,
    inventory_patterns: Vec<MeasurementInventoryPattern>,
    sites: Vec<DerivedMeasurementSite>,
    mutation_path: &'static str,
    mutation_needle: &'static str,
    planted_unregistered_copy: &'static str,
}

fn canonical_measurement(results: &str, threshold: &str) -> Result<CanonicalMeasurement, String> {
    let rows = results
        .lines()
        .filter_map(|line| {
            let cells = line.split('|').map(str::trim).collect::<Vec<_>>();
            (cells.get(1) == Some(&threshold)).then_some(cells)
        })
        .collect::<Vec<_>>();
    if rows.len() != 1 {
        return Err(format!(
            "canonical threshold {threshold:?} must occur exactly once, found {}",
            rows.len()
        ));
    }
    let outcome = rows[0]
        .get(2)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing outcome for canonical threshold {threshold:?}"))?;
    let measurement = rows[0]
        .get(3)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!("missing final measurement for canonical threshold {threshold:?}")
        })?;
    Ok(CanonicalMeasurement {
        outcome: (*outcome).to_string(),
        measurement: (*measurement).to_string(),
    })
}

fn collect_measurement_documents(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("policy-lint: cannot read {}: {err}", rel_display(dir)));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        let file_type = entry.file_type().expect("file type");
        if file_type.is_dir() {
            collect_measurement_documents(&path, out);
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "md" || extension == "json")
        {
            out.push(path);
        }
    }
}

fn measurement_documents() -> BTreeMap<String, String> {
    let root = repo_root();
    let mut paths = vec![root.join("CHANGELOG.md")];
    collect_measurement_documents(&root.join("docs"), &mut paths);
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let relative = rel_display(&path);
            let content = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("cannot read {relative}: {err}"));
            (relative, content)
        })
        .collect()
}

fn spike_measurement_specs() -> Vec<MeasurementSpec> {
    const FEASIBILITY: &str = "docs/specs/ADR-046-feasibility-and-spikes.md";
    const STORE: &str = "docs/specs/ADR-046-resource-store-redb.md";
    const DECISIONS: &str = "docs/specs/ADR-046-decision-register.md";
    const VALIDATION: &str = "docs/specs/ADR-046-validation-and-delivery.md";
    const WORK_ITEMS: &str = "docs/specs/ADR-046-work-items.json";

    let spike_01_summary_sites = || {
        vec![
            DerivedMeasurementSite {
                path: "CHANGELOG.md",
                expectation: DerivedExpectation::OutcomeSummary(
                    "Functional, watch, conflict, crash-recovery",
                ),
                copies: 1,
            },
            DerivedMeasurementSite {
                path: FEASIBILITY,
                expectation: DerivedExpectation::OutcomeSummary(
                    "functional, watch, conflict, crash-recovery, and commit-to-handler thresholds passed",
                ),
                copies: 1,
            },
            DerivedMeasurementSite {
                path: FEASIBILITY,
                expectation: DerivedExpectation::OutcomeSummary(
                    "functional/index/revision/watch/group-commit/crash-recovery passed",
                ),
                copies: 1,
            },
            DerivedMeasurementSite {
                path: FEASIBILITY,
                expectation: DerivedExpectation::OutcomeSummary(
                    "Functional scale, watch correctness",
                ),
                copies: 1,
            },
            DerivedMeasurementSite {
                path: FEASIBILITY,
                expectation: DerivedExpectation::OutcomeSummary(
                    "SPIKE-01 functional scale, watch correctness",
                ),
                copies: 1,
            },
            DerivedMeasurementSite {
                path: STORE,
                expectation: DerivedExpectation::OutcomeSummary(
                    "Functional, crash, watch, conflict, and commit-to-handler thresholds passed",
                ),
                copies: 1,
            },
            DerivedMeasurementSite {
                path: DECISIONS,
                expectation: DerivedExpectation::OutcomeSummary(
                    "Functional, watch, conflict, crash-recovery, and commit-to-handler thresholds passed",
                ),
                copies: 1,
            },
            DerivedMeasurementSite {
                path: VALIDATION,
                expectation: DerivedExpectation::OutcomeSummary(
                    "Functional, watch, conflict, crash-recovery, and latency thresholds passed",
                ),
                copies: 1,
            },
            DerivedMeasurementSite {
                path: WORK_ITEMS,
                expectation: DerivedExpectation::OutcomeSummary(
                    "SPIKE-01 functional scale, watch correctness",
                ),
                copies: 1,
            },
        ]
    };

    vec![
        MeasurementSpec {
            name: "10k correctness",
            threshold: "10,000 resources, 5 runs, zero oracle divergence",
            expected_outcome: "MEASURED-PASS",
            fingerprint: "5/5 runs",
            inventory_patterns: vec![MeasurementInventoryPattern {
                description: "five-run pass ratio",
                regex: r"(?i)\b(?:5/5|5\s+(?:of|out\s+of)\s+5|all\s+(?:5|five))\s+runs?\b",
                copies: 0,
            }],
            sites: spike_01_summary_sites(),
            mutation_path: FEASIBILITY,
            mutation_needle: "Functional scale",
            planted_unregistered_copy: "Independent result: all five runs passed.",
        },
        MeasurementSpec {
            name: "watch no-gap",
            threshold: "100 watches, no misses, duplicates, or gaps",
            expected_outcome: "MEASURED-PASS",
            fingerprint: "21,866 exact ChangeBatch comparisons",
            inventory_patterns: vec![MeasurementInventoryPattern {
                description: "21,866 comparisons",
                regex: r"(?i)\b21,?866(?:\s+\S+){0,3}\s+comparisons?\b",
                copies: 0,
            }],
            sites: spike_01_summary_sites(),
            mutation_path: FEASIBILITY,
            mutation_needle: "watch correctness",
            planted_unregistered_copy: "Independent result: 21,866 watch comparisons passed.",
        },
        MeasurementSpec {
            name: "group commit",
            threshold: "More than half of non-conflicting storm writes use a batch larger than 1",
            expected_outcome: "MEASURED-PASS",
            fingerprint: "48/50, 96%",
            inventory_patterns: vec![MeasurementInventoryPattern {
                description: "48-of-50 group-commit denominator",
                regex: r"\b(?:48/50|48\s+(?:of|out\s+of)\s+50)\b",
                copies: 4,
            }],
            sites: vec![
                DerivedMeasurementSite {
                    path: "CHANGELOG.md",
                    expectation: DerivedExpectation::CanonicalMeasurement,
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: FEASIBILITY,
                    expectation: DerivedExpectation::CanonicalMeasurement,
                    copies: 2,
                },
                DerivedMeasurementSite {
                    path: WORK_ITEMS,
                    expectation: DerivedExpectation::CanonicalMeasurement,
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: FEASIBILITY,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "functional, watch, conflict, crash-recovery, and commit-to-handler thresholds passed",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: FEASIBILITY,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "functional/index/revision/watch/group-commit/crash-recovery passed",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: STORE,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "Functional, crash, watch, conflict, and commit-to-handler thresholds passed",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: DECISIONS,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "Functional, watch, conflict, crash-recovery, and commit-to-handler thresholds passed",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: VALIDATION,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "Functional, watch, conflict, crash-recovery, and latency thresholds passed",
                    ),
                    copies: 1,
                },
            ],
            mutation_path: "CHANGELOG.md",
            mutation_needle: "48/50, 96%",
            planted_unregistered_copy: "Independent group commit result: 48 of 50 writes batched.",
        },
        MeasurementSpec {
            name: "crash boundaries",
            threshold: "All 13 crash boundaries recover atomically or refuse to open",
            expected_outcome: "MEASURED-PASS",
            fingerprint: "13/13",
            inventory_patterns: vec![MeasurementInventoryPattern {
                description: "13 crash boundaries",
                regex: r"(?i)\b(?:(?:all\s+)?(?:13|thirteen)|13/13)\s+crash(?:[- ]recovery)?\s+boundar(?:y|ies)\b",
                copies: 3,
            }],
            sites: vec![
                DerivedMeasurementSite {
                    path: "CHANGELOG.md",
                    expectation: DerivedExpectation::OutcomeSummary(
                        "Functional, watch, conflict, crash-recovery",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: FEASIBILITY,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "functional, watch, conflict, crash-recovery, and commit-to-handler thresholds passed",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: FEASIBILITY,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "functional/index/revision/watch/group-commit/crash-recovery passed",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: FEASIBILITY,
                    expectation: DerivedExpectation::CanonicalFragment {
                        source: "13/13",
                        derived: "all 13 crash boundaries",
                    },
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: FEASIBILITY,
                    expectation: DerivedExpectation::CanonicalFragment {
                        source: "13/13",
                        derived: "13/13 crash boundaries",
                    },
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: STORE,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "Functional, crash, watch, conflict, and commit-to-handler thresholds passed",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: DECISIONS,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "Functional, watch, conflict, crash-recovery, and commit-to-handler thresholds passed",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: VALIDATION,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "Functional, watch, conflict, crash-recovery, and latency thresholds passed",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: WORK_ITEMS,
                    expectation: DerivedExpectation::CanonicalFragment {
                        source: "13/13",
                        derived: "13/13 crash boundaries",
                    },
                    copies: 1,
                },
            ],
            mutation_path: WORK_ITEMS,
            mutation_needle: "13/13 crash boundaries",
            planted_unregistered_copy: "Independent result: all 13 crash boundaries passed.",
        },
        MeasurementSpec {
            name: "median RSS",
            threshold: "Median whole-process maximum RSS at or below 24 MiB",
            expected_outcome: "MEASURED-FAIL",
            fingerprint: "25,216 KiB (24.625 MiB), 640 KiB or about 2.6% above 24,576 KiB",
            inventory_patterns: vec![
                MeasurementInventoryPattern {
                    description: "25,216 KiB whole-process RSS",
                    regex: r"(?i)\b25,?216\s+KiB\b",
                    copies: 11,
                },
                MeasurementInventoryPattern {
                    description: "24.625 MiB whole-process RSS",
                    regex: r"(?i)\b24\.625\s+MiB\b",
                    copies: 10,
                },
                MeasurementInventoryPattern {
                    description: "640 KiB threshold excess",
                    regex: r"(?i)\b640\s+KiB\b",
                    copies: 10,
                },
                MeasurementInventoryPattern {
                    description: "2.6 percent excess over 24,576 KiB",
                    regex: r"(?i)\b2\.6%\s+above\s+24,?576\s+KiB\b",
                    copies: 10,
                },
            ],
            sites: vec![
                DerivedMeasurementSite {
                    path: "CHANGELOG.md",
                    expectation: DerivedExpectation::CanonicalMeasurement,
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: FEASIBILITY,
                    expectation: DerivedExpectation::CanonicalMeasurement,
                    copies: 4,
                },
                DerivedMeasurementSite {
                    path: STORE,
                    expectation: DerivedExpectation::CanonicalMeasurement,
                    copies: 2,
                },
                DerivedMeasurementSite {
                    path: DECISIONS,
                    expectation: DerivedExpectation::CanonicalMeasurement,
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: VALIDATION,
                    expectation: DerivedExpectation::CanonicalMeasurement,
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: WORK_ITEMS,
                    expectation: DerivedExpectation::CanonicalMeasurement,
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: FEASIBILITY,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "SPIKE-01's whole-process RSS failure",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: STORE,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "SPIKE-01 failed the whole-process RSS",
                    ),
                    copies: 2,
                },
                DerivedMeasurementSite {
                    path: STORE,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "SPIKE-01 executed and failed the whole-process RSS threshold",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: STORE,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "SPIKE-01 executed but failed the whole-process RSS threshold",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: WORK_ITEMS,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "SPIKE-01 failed the whole-process RSS threshold",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: WORK_ITEMS,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "SPIKE-01 executed and failed the whole-process RSS threshold",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: WORK_ITEMS,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "SPIKE-01 executed but failed the whole-process RSS threshold",
                    ),
                    copies: 1,
                },
            ],
            mutation_path: STORE,
            mutation_needle: "25,216 KiB",
            planted_unregistered_copy: "Independent RSS result: 25,216 KiB.",
        },
        MeasurementSpec {
            name: "SPIKE-02 p95",
            threshold: "Commit-to-handler p95 at or below 5,000 us in all profiles",
            expected_outcome: "MEASURED-PASS",
            fingerprint: "115.043 us / 116.195 us / 128.902 us",
            inventory_patterns: vec![
                MeasurementInventoryPattern {
                    description: "115.043 us p95",
                    regex: r"\b115\.043\s+us\b",
                    copies: 3,
                },
                MeasurementInventoryPattern {
                    description: "116.195 us p95",
                    regex: r"\b116\.195\s+us\b",
                    copies: 3,
                },
                MeasurementInventoryPattern {
                    description: "128.902 us p95",
                    regex: r"\b128\.902\s+us\b",
                    copies: 3,
                },
            ],
            sites: vec![
                DerivedMeasurementSite {
                    path: FEASIBILITY,
                    expectation: DerivedExpectation::CanonicalMeasurement,
                    copies: 2,
                },
                DerivedMeasurementSite {
                    path: WORK_ITEMS,
                    expectation: DerivedExpectation::CanonicalMeasurement,
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: "CHANGELOG.md",
                    expectation: DerivedExpectation::OutcomeSummary("latency thresholds passed"),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: FEASIBILITY,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "commit-to-handler thresholds passed",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: FEASIBILITY,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "all three SPIKE-02 profiles passed",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: STORE,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "commit-to-handler thresholds passed",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: STORE,
                    expectation: DerivedExpectation::OutcomeSummary("SPIKE-02 passed"),
                    copies: 2,
                },
                DerivedMeasurementSite {
                    path: DECISIONS,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "commit-to-handler thresholds passed",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: VALIDATION,
                    expectation: DerivedExpectation::OutcomeSummary("latency thresholds passed"),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: WORK_ITEMS,
                    expectation: DerivedExpectation::OutcomeSummary("SPIKE-02 passed"),
                    copies: 1,
                },
            ],
            mutation_path: FEASIBILITY,
            mutation_needle: "115.043 us / 116.195 us / 128.902 us",
            planted_unregistered_copy: "Independent p95 result: 115.043 us.",
        },
        MeasurementSpec {
            name: "SPIKE-02 p99",
            threshold: "Commit-to-handler p99 reported; document any value above 20 ms",
            expected_outcome: "MEASURED-PASS",
            fingerprint: "134.834 us / 140.928 us / 1,009.871 us; none exceeded 20 ms",
            inventory_patterns: vec![
                MeasurementInventoryPattern {
                    description: "134.834 us p99",
                    regex: r"\b134\.834\s+us\b",
                    copies: 3,
                },
                MeasurementInventoryPattern {
                    description: "140.928 us p99",
                    regex: r"\b140\.928\s+us\b",
                    copies: 3,
                },
                MeasurementInventoryPattern {
                    description: "1,009.871 us p99",
                    regex: r"\b1,?009\.871\s+us\b",
                    copies: 3,
                },
                MeasurementInventoryPattern {
                    description: "no p99 value exceeded 20 ms",
                    regex: r"(?i)\bnone\s+exceeded\s+20\s+ms\b",
                    copies: 3,
                },
            ],
            sites: vec![
                DerivedMeasurementSite {
                    path: FEASIBILITY,
                    expectation: DerivedExpectation::CanonicalMeasurement,
                    copies: 2,
                },
                DerivedMeasurementSite {
                    path: WORK_ITEMS,
                    expectation: DerivedExpectation::CanonicalMeasurement,
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: "CHANGELOG.md",
                    expectation: DerivedExpectation::OutcomeSummary("latency thresholds passed"),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: FEASIBILITY,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "commit-to-handler thresholds passed",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: STORE,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "commit-to-handler thresholds passed",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: STORE,
                    expectation: DerivedExpectation::OutcomeSummary("SPIKE-02 passed"),
                    copies: 2,
                },
                DerivedMeasurementSite {
                    path: DECISIONS,
                    expectation: DerivedExpectation::OutcomeSummary(
                        "commit-to-handler thresholds passed",
                    ),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: VALIDATION,
                    expectation: DerivedExpectation::OutcomeSummary("latency thresholds passed"),
                    copies: 1,
                },
                DerivedMeasurementSite {
                    path: WORK_ITEMS,
                    expectation: DerivedExpectation::OutcomeSummary("SPIKE-02 passed"),
                    copies: 1,
                },
            ],
            mutation_path: WORK_ITEMS,
            mutation_needle: "134.834 us / 140.928 us / 1,009.871 us",
            planted_unregistered_copy: "Independent p99 result: 134.834 us.",
        },
    ]
}

fn validate_spike_measurement(
    spec: &MeasurementSpec,
    canonical: &CanonicalMeasurement,
    documents: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut errors = Vec::new();
    if canonical.outcome != spec.expected_outcome {
        errors.push(format!(
            "{}: canonical outcome is {:?}, expected {:?}",
            spec.name, canonical.outcome, spec.expected_outcome
        ));
    }
    if !canonical.measurement.contains(spec.fingerprint) {
        errors.push(format!(
            "{}: canonical measurement {:?} no longer contains discovery fingerprint {:?}",
            spec.name, canonical.measurement, spec.fingerprint
        ));
    }

    for site in &spec.sites {
        let Some(content) = documents.get(site.path) else {
            errors.push(format!("{}: missing derived site {}", spec.name, site.path));
            continue;
        };
        let needle = match site.expectation {
            DerivedExpectation::CanonicalMeasurement => canonical.measurement.as_str(),
            DerivedExpectation::CanonicalFragment { source, derived } => {
                if !canonical.measurement.contains(source) {
                    errors.push(format!(
                        "{}: canonical measurement {:?} no longer supports derived fragment {:?}",
                        spec.name, canonical.measurement, derived
                    ));
                }
                derived
            }
            DerivedExpectation::OutcomeSummary(summary) => summary,
        };
        let actual = normalized_whitespace(content)
            .matches(&normalized_whitespace(needle))
            .count();
        if actual != site.copies {
            errors.push(format!(
                "{}: {} must contain {:?} exactly {} time(s), found {actual}",
                spec.name, site.path, needle, site.copies
            ));
        }
    }

    for pattern in &spec.inventory_patterns {
        let regex = Regex::new(pattern.regex).expect("valid measurement inventory regex");
        let occurrences = documents
            .iter()
            .filter_map(|(path, content)| {
                let count = regex.find_iter(&normalized_whitespace(content)).count();
                (count > 0).then_some((path, count))
            })
            .collect::<Vec<_>>();
        let actual = occurrences.iter().map(|(_, count)| count).sum::<usize>();
        if actual != pattern.copies {
            let locations = occurrences
                .iter()
                .map(|(path, count)| format!("{path} ({count})"))
                .collect::<Vec<_>>()
                .join(", ");
            errors.push(format!(
                "{}: global docs/** and CHANGELOG.md inventory for {} ({:?}) must contain exactly {} copy/copies, found {actual} at [{}]; register or remove every new measurement-shaped copy",
                spec.name,
                pattern.description,
                pattern.regex,
                pattern.copies,
                locations
            ));
        }
    }

    errors
}

fn validate_spike_measurements(results: &str, documents: &BTreeMap<String, String>) -> Vec<String> {
    let specs = spike_measurement_specs();
    let canonical_row_count = results
        .lines()
        .filter(|line| line.contains("| MEASURED-"))
        .count();
    let mut errors = Vec::new();
    if canonical_row_count != specs.len() {
        errors.push(format!(
            "canonical final-threshold table has {canonical_row_count} measurement rows but the guard registers {}",
            specs.len()
        ));
    }
    for spec in &specs {
        match canonical_measurement(results, spec.threshold) {
            Ok(canonical) => {
                errors.extend(validate_spike_measurement(spec, &canonical, documents));
            }
            Err(error) => errors.push(format!("{}: {error}", spec.name)),
        }
    }
    errors
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
fn d103_rejects_a_conformant_prefix_carrying_trailing_or_leading_junk() {
    // A conformant 24-byte instant with junk glued directly onto it. Validating
    // only the conformant prefix accepts it; the token is now validated whole, so
    // the trailing junk is rejected.
    assert!(
        !scan_d103("f.md", "createdAt: 2026-07-22T00:00:00.000Zjunk").is_empty(),
        "a conformant prefix with trailing junk must be rejected"
    );
    // Leading junk fused to the year is rejected the same way.
    assert!(
        !scan_d103("f.md", "createdAt: x2026-07-22T00:00:00.000Z").is_empty(),
        "a conformant instant with leading junk must be rejected"
    );
    // A well-formed but impossible calendar value is rejected by the semantic
    // check, not merely the shape check.
    for bad in [
        "createdAt: 2026-02-30T00:00:00.000Z", // Feb 30 never exists
        "createdAt: 2026-07-22T23:59:60.000Z", // :60 leap second
    ] {
        assert!(
            !scan_d103("f.md", bad).is_empty(),
            "an impossible calendar instant must be rejected: {bad:?}"
        );
    }
    // The exact conformant instant, delimited by whitespace/quotes/commas, is
    // still accepted - extension stops at the first non-alphanumeric byte.
    for ok in [
        "createdAt: 2026-07-22T00:00:00.000Z",
        "createdAt: \"2026-07-22T00:00:00.000Z\"",
        "createdAt: 2026-07-22T00:00:00.000Z, next: 1",
    ] {
        assert!(
            scan_d103("f.md", ok).is_empty(),
            "the exact instant must be accepted: {ok:?}"
        );
    }
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
fn d104_validates_quoted_json_and_indented_nix_type_fields() {
    // A `^type`-anchored regex sees only a bare, zero-indent YAML `type:`. A
    // quoted JSON `"type"` and an indented Nix `type =` slip through unvalidated
    // under that approach. The structural pass reads all three identically.
    let bad_json = concat!(
        "```json\n",
        "{\n",
        "  \"apiVersion\": \"resources.d2bus.org/v3\",\n",
        "  \"type\": \"Widget\",\n",
        "  \"metadata\": { \"name\": \"w\" }\n",
        "}\n",
        "```\n",
    );
    assert!(
        !scan_d104("f.md", bad_json).is_empty(),
        "an unknown quoted JSON type in an envelope must be flagged"
    );
    let bad_nix = concat!(
        "```nix\n",
        "d2b.zones.\"z\".resources.\"w\" = {\n",
        "  type = \"Widget\";\n",
        "  spec = { size = 1; };\n",
        "};\n",
        "```\n",
    );
    assert!(
        !scan_d104("f.md", bad_nix).is_empty(),
        "an unknown indented Nix type in a resource declaration must be flagged"
    );
    // A `resourceType` field in a JSON envelope is validated the same way.
    let bad_resource_type = concat!(
        "```json\n",
        "{\n",
        "  \"apiVersion\": \"resources.d2bus.org/v3\",\n",
        "  \"resourceType\": \"Widget\"\n",
        "}\n",
        "```\n",
    );
    assert!(
        !scan_d104("f.md", bad_resource_type).is_empty(),
        "an unknown quoted JSON resourceType in an envelope must be flagged"
    );
    // Standard types in the same quoted/indented shapes are accepted.
    let good_json = concat!(
        "```json\n",
        "{\n",
        "  \"apiVersion\": \"resources.d2bus.org/v3\",\n",
        "  \"type\": \"Host\"\n",
        "}\n",
        "```\n",
    );
    let good_nix = concat!(
        "```nix\n",
        "d2b.zones.\"z\".resources.\"h\" = {\n",
        "  type = \"Guest\";\n",
        "  spec = { };\n",
        "};\n",
        "```\n",
    );
    for ok in [good_json, good_nix] {
        assert!(
            scan_d104("f.md", ok).is_empty(),
            "a standard type in a quoted/indented field must be accepted: {ok:?}"
        );
    }
    // A `<ResourceType>` placeholder in a Nix declaration is authored
    // deliberately and is not flagged.
    let placeholder = concat!(
        "```nix\n",
        "d2b.zones.\"z\".resources.\"n\" = {\n",
        "  type = \"<ResourceType>\";\n",
        "  spec = { };\n",
        "};\n",
        "```\n",
    );
    assert!(
        scan_d104("f.md", placeholder).is_empty(),
        "a `<ResourceType>` placeholder must not be flagged"
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
    // An unrecognised non-decimal token in value position (a pseudo-scientific
    // literal, or a bare word) must be rejected. A classify-then-fall-through
    // reader silently accepts anything it cannot classify.
    assert!(
        !scan_d108("f.md", "retryAfterMs: 1e3").is_empty(),
        "a scientific-notation literal must be rejected"
    );
    assert!(
        !scan_d108("f.md", "retryAfterMs: banana").is_empty(),
        "a bare word must be rejected"
    );
    assert!(
        !scan_d108("f.md", "retryAfterMs: nonsense").is_empty(),
        "a bare word must be rejected"
    );
    assert!(
        !scan_d108("f.md", "retryAfterMs: 0x1f4").is_empty(),
        "a hexadecimal literal is not a bare decimal"
    );
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
fn parser_backed_scanners_reject_structural_bypasses() {
    // Each structural counterexample below is asserted rejected through the
    // public scanner, so a regression that reopens a bypass fails here.

    // D103: a conformant instant with a `_`-suffixed junk token. The candidate
    // token is extended over `_`, so the WHOLE token is judged, not the 24-byte
    // conformant prefix.
    assert!(
        !scan_d103("f.md", "createdAt: 2026-07-22T00:00:00.000Z_junk").is_empty(),
        "a conformant prefix with a `_junk` suffix must be rejected"
    );

    // D103: a JSON key and value split across two lines. The per-line passes
    // cannot see the pair; the structural pass validates the complete parsed
    // scalar of the `createdAt` field.
    let d103_split = "```json\n{\n  \"createdAt\":\n    \"2026-07-22\"\n}\n```";
    assert!(
        !scan_d103("f.md", d103_split).is_empty(),
        "a createdAt key/value split across lines must be rejected structurally"
    );

    // D104: an over-qualified `acme.d2bus.org.Widget.Type`. The substring pass
    // validates only the `...Widget` prefix and accepts it; the structural pass
    // validates the COMPLETE scalar and rejects the trailing `.Type`.
    let d104_overqualified = "```json\n{ \"apiVersion\": \"resources.d2bus.org/v3\", \"type\": \"acme.d2bus.org.Widget.Type\" }\n```";
    assert!(
        !scan_d104("f.md", d104_overqualified).is_empty(),
        "an over-qualified d2bus.org type must be rejected on the complete scalar"
    );

    // D108: `retryAfterMs: NaN` is a non-finite float value, not a type
    // annotation; the previous upper-camel guard swallowed it.
    assert!(
        !scan_d108("f.md", "retryAfterMs: NaN").is_empty(),
        "retryAfterMs: NaN must be rejected as a value, not accepted as a type"
    );

    // D108: a quoted JSON key `"retryAfterMs"` the per-line assignment regex can
    // never match (a `\"` sits between the key and the `:`). The structural pass
    // reads the parsed key and rejects the `"5s"` duration value.
    let d108_quoted_key = "```json\n{ \"retryAfterMs\": \"5s\" }\n```";
    assert!(
        !scan_d108("f.md", d108_quoted_key).is_empty(),
        "a quoted JSON retryAfterMs key with a duration value must be rejected"
    );

    // D108: a JSON key and value split across two lines. The structural pass
    // sees the pair; the per-line passes cannot.
    let d108_split = "```json\n{\n  \"retryAfterMs\":\n    \"5s\"\n}\n```";
    assert!(
        !scan_d108("f.md", d108_split).is_empty(),
        "a retryAfterMs key/value split across lines must be rejected structurally"
    );
}

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

#[test]
fn spike_measurement_contracts_match_and_reject_mutations() {
    let root = repo_root();
    let results_path = root.join("proofs/redb-resource-store-spike/RESULTS.md");
    let results = std::fs::read_to_string(&results_path)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", rel_display(&results_path)));
    let documents = measurement_documents();
    let errors = validate_spike_measurements(&results, &documents);
    assert!(errors.is_empty(), "{}", errors.join("\n"));

    let feasibility =
        std::fs::read_to_string(root.join("docs/specs/ADR-046-feasibility-and-spikes.md"))
            .expect("read feasibility spec");
    assert!(feasibility.contains(
        "cargo build --release --locked --manifest-path \
         proofs/redb-resource-store-spike/Cargo.toml --bin rss-fixture"
    ));
    assert!(feasibility.contains(
        "time -v proofs/redb-resource-store-spike/target/release/rss-fixture \
         --resources 10000 --watches 100"
    ));

    for spec in spike_measurement_specs() {
        let canonical = canonical_measurement(&results, spec.threshold)
            .unwrap_or_else(|error| panic!("{}: {error}", spec.name));
        let original = documents
            .get(spec.mutation_path)
            .unwrap_or_else(|| panic!("missing mutation site {}", spec.mutation_path));
        assert!(
            original.contains(spec.mutation_needle),
            "{} mutation needle {:?} is absent from {}",
            spec.name,
            spec.mutation_needle,
            spec.mutation_path
        );
        let mut mutated = documents.clone();
        mutated.insert(
            spec.mutation_path.to_string(),
            original.replacen(spec.mutation_needle, "[mutated measurement]", 1),
        );
        let errors = validate_spike_measurement(&spec, &canonical, &mutated);
        assert!(
            !errors.is_empty(),
            "{} guard accepted a perturbed derived copy in {}",
            spec.name,
            spec.mutation_path
        );
    }

    const UNREGISTERED_DOCUMENT: &str = "docs/explanation/unregistered-spike-copy.md";
    assert!(
        !documents.contains_key(UNREGISTERED_DOCUMENT),
        "plant path must not replace a real documentation file"
    );

    for spec in spike_measurement_specs() {
        assert!(
            spec.sites
                .iter()
                .all(|site| site.path != UNREGISTERED_DOCUMENT),
            "{} plant path is unexpectedly registered",
            spec.name
        );
        let canonical = canonical_measurement(&results, spec.threshold)
            .unwrap_or_else(|error| panic!("{}: {error}", spec.name));
        let mut mutated = documents.clone();
        mutated.insert(
            UNREGISTERED_DOCUMENT.to_string(),
            spec.planted_unregistered_copy.to_string(),
        );
        let errors = validate_spike_measurement(&spec, &canonical, &mutated);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("global docs/** and CHANGELOG.md inventory")),
            "{} global inventory accepted unregistered copy {:?}; errors: {}",
            spec.name,
            spec.planted_unregistered_copy,
            errors.join("\n")
        );
    }
}

#[test]
fn redb_dependency_is_isolated_to_the_proof_workspace() {
    let root = repo_root();
    let main_manifest =
        std::fs::read_to_string(root.join("packages/Cargo.toml")).expect("read main manifest");
    let main_lock =
        std::fs::read_to_string(root.join("packages/Cargo.lock")).expect("read main lockfile");
    let contract_manifest =
        std::fs::read_to_string(root.join("packages/d2b-resource-store-redb/Cargo.toml"))
            .expect("read contract manifest");
    let schema =
        std::fs::read_to_string(root.join("packages/d2b-resource-store-redb/src/schema.rs"))
            .expect("read table schema");
    let proof_manifest =
        std::fs::read_to_string(root.join("proofs/redb-resource-store-spike/Cargo.toml"))
            .expect("read proof manifest");
    let proof_lock =
        std::fs::read_to_string(root.join("proofs/redb-resource-store-spike/Cargo.lock"))
            .expect("read proof lockfile");

    assert!(!main_manifest.contains("\nredb ="));
    assert!(!main_lock.contains("\nname = \"redb\"\n"));
    assert!(!contract_manifest.contains("\nredb"));
    assert!(!schema.contains("redb::"));
    assert!(!schema.contains("TableDefinition"));
    assert!(proof_manifest.contains("redb = { version = \"=4.1.0\""));
    assert!(proof_lock.contains("\nname = \"redb\"\n"));
}
