//! Blocking-API census (U32, issue #524; per-crate + allow inventory + CI
//! cap, U2).
//!
//! Counts the workspace's uses of every API on the `clippy.toml`
//! `disallowed-methods` deny list, separating production from test contexts.
//! The deny list is the single source of truth - whatever it names is what
//! this counts - so the two cannot drift. The lint is the gate; this is the
//! meter.
//!
//! Two counting paths keep the meter on the lint's predicate (plan R10:
//! clippy deny is the flip authority). The authoritative count for EVERY
//! deny-entry class comes from `cargo clippy --message-format=json`
//! `disallowed_methods` diagnostics (run with the manifest's documented
//! de-escalation set - `-W warnings -W clippy::disallowed_methods
//! -W clippy::await_holding_lock -W clippy::await_holding_refcell_ref` - so
//! the manifest's `deny` level cannot fail the run while `#[allow]`
//! attributes keep suppressing - the same predicate the lint enforces under
//! deny). The lexical meter is the report's cross-check:
//!
//! * Free-function and module-qualified entries (`std::fs::read_to_string`)
//!   are counted by their FULL configured path text, so a tokio-replacement
//!   line (`tokio::fs::read_to_string`) never counts against the `std::fs`
//!   entry, and `std::fs::read` never matches a `read_to_string` line.
//! * Instance-method entries (`std::sync::Mutex::lock`, `Receiver::recv`)
//!   are invisible to path-text matching (a call site is `mutex.lock()`), so
//!   the clippy diagnostics are their only measurable count.
//!
//! The clippy-derived count is authoritative because the lexical meter both
//! under- and over-counts: it cannot see imported bare calls
//! (`read(...)` after `use std::fs::read`), and it counts doc-comment
//! mentions of a denied path as uses. The baseline the CI cap compares
//! against therefore stores the clippy-derived counts, and a new
//! `spawn_blocking` call is caught no matter what comments say.
//!
//! A diagnostic is charged to a crate only when one of its spans renders the
//! call the lint names. That condition is not cosmetic: a macro-generated
//! call spans the macro's own tokens, so `#[tokio::test]` - which expands to
//! `Runtime::block_on(async { .. })` via `quote_spanned!` - reports its
//! `block_on` against the test body's last statement, an `assert_eq!` line.
//! Charging that span billed each async test one phantom `Runtime::block_on`
//! to whichever crate held the test, moving counts on code whose call sites
//! never changed. Macro-generated and otherwise unattributable diagnostics
//! are still reported, so the number stays visible, but they never enter a
//! crate's count: an unattributable hit must not be able to move a committed
//! baseline line.
//!
//! The committed baseline therefore carries TWO ratchet axes per crate, and
//! a change cannot lower either one to hide a call site:
//!
//! * the clippy-derived per-entry counts, which may not grow, and
//! * the number of `#[allow(clippy::disallowed_methods)]` /
//!   `#[expect(clippy::disallowed_methods)]` sites in the crate, which may
//!   not grow either.
//!
//! The second axis is what makes the first one a ratchet instead of a
//! suggestion. An `#[allow]` outside a provider crate silently deletes the
//! diagnostic the count is derived from, so a count-only baseline can be
//! walked down one site per attribute - to zero - while every gate stays
//! green. With the suppression axis attached, lowering a count is only
//! possible by removing the call: a count that drops while a new suppression
//! appears is exactly the walk-down, and it fails.
//!
//! The census also inventories every `#[allow]`/`#[expect]` suppression of a
//! banned-API lint per crate, splitting module-level blanket allows
//! (`#![allow(...)]`) from per-site allows (`#[allow(...)]`); the crate
//! policy check (`provider_crate_policy`) gates the reasons.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// The banned-API lint family the census inventories and the policy check
/// gates: the deny-list lints plus the two live denials.
pub const BANNED_API_LINTS: &[&str] = &[
    "clippy::disallowed_methods",
    "clippy::disallowed_types",
    "clippy::await_holding_lock",
    "clippy::await_holding_refcell_ref",
];

/// The lint the census's authoritative counts are derived from. It is also
/// the one suppression family the baseline ratchets separately: silencing it
/// is the only way to move a count without moving a call site, so its
/// `#[allow]`/`#[expect]` sites are a ratchet axis of their own.
pub const COUNTING_LINT: &str = "clippy::disallowed_methods";

/// How one deny-list entry is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    /// Free function / module-qualified call: the full configured path text
    /// appears at call and import sites, so the lexical meter counts it.
    Textual,
    /// Method called through an instance or type expression (`mutex.lock()`,
    /// `File::open(...)`): invisible to path-text matching; counted from
    /// clippy diagnostics.
    InstanceMethod,
}

/// One entry from the deny list: the fully-qualified API path, the bare tail
/// clippy matches on (everything after the final `::`), and the counting
/// class.
pub struct DeniedApi {
    /// Fully-qualified API path as configured, e.g. `std::sync::Mutex::lock`.
    pub path: String,
    /// The bare tail clippy matches on: everything after the final `::`.
    pub tail: String,
    /// How the entry is counted: textually or from clippy diagnostics.
    pub kind: EntryKind,
}

/// Parse the `path = "..."` entries of a clippy.toml `disallowed-methods` list.
pub fn parse_deny_list(clippy_toml: &str) -> Vec<DeniedApi> {
    let mut entries = Vec::new();
    for line in clippy_toml.lines() {
        let Some(start) = line.find("path = \"") else {
            continue;
        };
        let rest = &line[start + "path = \"".len()..];
        let Some(end) = rest.find('"') else {
            continue;
        };
        let path = rest[..end].to_owned();
        // clippy resolves the configured path to a method; the nearest
        // distinctive text form is the last two segments (`socket::connect`
        // rather than a bare `connect`, which collides with every adapter's
        // async connect). A path with one segment keeps that segment.
        let segments: Vec<&str> = path.split("::").collect();
        let tail = if segments.len() >= 2 {
            format!("{}::{}", segments[segments.len() - 2], segments[segments.len() - 1])
        } else {
            path.clone()
        };
        // A type-qualified entry (`std::sync::Mutex::lock`) is called through
        // an instance or type expression; a module-qualified entry
        // (`std::fs::read_to_string`) is a free function whose full path text
        // stays visible. The type segment is the one before the method name.
        let kind = if segments.len() >= 3
            && segments[segments.len() - 2]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase())
        {
            EntryKind::InstanceMethod
        } else {
            EntryKind::Textual
        };
        entries.push(DeniedApi { path, tail, kind });
    }
    entries
}

/// One context's lines: `(line_number, trimmed_text)`.
pub type ContextLines = Vec<(usize, String)>;

/// A source file split into its production and test contexts.
pub type SplitContext = (ContextLines, ContextLines);

/// Split a source file's lines into production and test contexts.
///
/// A file is test context wholesale when its path lives in a `tests/`,
/// `integration/`, or `benches/` directory. Otherwise, lines inside a
/// `#[cfg(test)]` block are test context; everything else is production.
pub fn split_contexts(raw: &str, path_is_test: bool) -> SplitContext {
    let mut production = Vec::new();
    let mut test = Vec::new();
    let mut cfg_test_depth: Option<isize> = None;
    let mut marker_line_pending = false;

    for (index, line) in raw.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();

        if cfg_test_depth.is_none() && trimmed.contains("#[cfg(test)]") {
            cfg_test_depth = Some(0);
            marker_line_pending = true;
        }

        let depth_here = trimmed.chars().filter(|c| *c == '{').count() as isize
            - trimmed.chars().filter(|c| *c == '}').count() as isize;

        if let Some(depth) = &mut cfg_test_depth {
            *depth += depth_here;
        }

        let is_test = path_is_test || cfg_test_depth.is_some();
        if is_test {
            test.push((line_number, trimmed.to_owned()));
        } else {
            production.push((line_number, trimmed.to_owned()));
        }

        if let Some(depth) = cfg_test_depth
            && depth <= 0
            && !marker_line_pending
        {
            cfg_test_depth = None;
        }
        marker_line_pending = false;
    }
    (production, test)
}

/// One entry's count across a file set, from the lexical meter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EntryCounts {
    pub production: usize,
    pub test: usize,
    /// Clippy-derived total (production + test), when the clippy scan ran.
    pub clippy: usize,
    /// First few production locations for the report.
    pub samples: Vec<String>,
}

/// Whether `text` contains `path` as a full configured-path token: the
/// character before and after the match must not be an identifier character,
/// so `std::fs::read` never matches `std::fs::read_to_string` and
/// `...::recv` never matches `...::recv_timeout`.
fn contains_full_path(text: &str, path: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(offset) = text[i..].find(path) {
        let start = i + offset;
        let end = start + path.len();
        let before_ok = start == 0 || !is_ident_char(bytes[start - 1]);
        let after_ok = end >= bytes.len() || !is_ident_char(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        i = start + 1;
    }
    false
}

fn is_ident_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Count each denied API's occurrences per context by FULL configured path
/// text. Only the exact configured path matches: `tokio::fs::read_to_string`
/// never counts against the `std::fs::read_to_string` entry, and
/// `Receiver::recv` never matches a `recv_timeout` line (the path text
/// continues past the entry's final segment).
pub fn count_occurrences(
    files: &[(String, SplitContext)],
    entries: &[DeniedApi],
) -> Vec<EntryCounts> {
    entries
        .iter()
        .map(|entry| {
            let mut counts = EntryCounts::default();
            for (path, (production, test)) in files {
                for (line, text) in production {
                    if contains_full_path(text, &entry.path) {
                        counts.production += 1;
                        if counts.samples.len() < 5 {
                            counts.samples.push(format!("{}:{}", path, line));
                        }
                    }
                }
                for (_, text) in test {
                    if contains_full_path(text, &entry.path) {
                        counts.test += 1;
                    }
                }
            }
            counts
        })
        .collect()
}

fn is_test_dir(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        name == "tests" || name == "integration" || name == "benches"
    })
}

/// One `disallowed_methods` diagnostic attributed to a deny-list entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClippyHit {
    pub entry_path: String,
    /// Repository-relative file path, as the clippy span names it.
    pub file: String,
    /// 1-based line of the call.
    pub line: usize,
    /// Whether the call was written at the attributed site or generated by a
    /// macro. A macro-generated call is reported separately and never
    /// charged to a crate: `#[tokio::test]` expands to
    /// `Runtime::block_on(async { .. })`, and the expansion's span points at
    /// the test body's last statement, so charging it to that line invents a
    /// call the author never wrote (see `parse_clippy_disallowed`).
    pub attributed: bool,
}

/// Whether a clippy diagnostic's primary span was produced by a macro
/// expansion rather than written at that line.
///
/// rustc marks a span that came from a macro body with a non-null
/// `expansion` block. `#[tokio::test]` builds its runtime with
/// `quote_spanned!`, so the `block_on` call it generates carries that
/// expansion - and its `file_name`/`line_start` land on the last statement of
/// the test body, not on a call site.
fn span_is_macro_generated(span: &serde_json::Value) -> bool {
    span.get("expansion")
        .map(|expansion| !expansion.is_null())
        .unwrap_or(false)
}

/// The source text a clippy span covers, concatenated across its `text`
/// entries. Empty when the stream carried no rendered text.
fn span_source_text(span: &serde_json::Value) -> String {
    span.get("text")
        .and_then(|text| text.as_array())
        .map(|lines| {
            lines
                .iter()
                .filter_map(|line| line.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Whether a span's rendered source contains the denied API's call token.
///
/// A textual entry is matched by its full configured path, so a
/// `tokio::fs::read_to_string` line never matches a `std::fs::read_to_string`
/// entry. An instance-method entry is matched by its method name applied to
/// a receiver - `state.lock()` - rather than by the configured `Type::method`
/// tail, which never appears in a call site. The method name alone is taken
/// as the last `::` segment, and a trailing `(` is required so `recv` never
/// matches `recv_timeout(`.
fn span_contains_call(span: &serde_json::Value, entry: &DeniedApi) -> bool {
    let text = span_source_text(span);
    if text.is_empty() {
        return false;
    }
    match entry.kind {
        EntryKind::Textual => contains_full_path(&text, &entry.path),
        EntryKind::InstanceMethod => {
            let method = entry.path.rsplit("::").next().unwrap_or(&entry.path);
            text.contains(&format!(".{method}("))
        }
    }
}

/// Parse `cargo clippy --message-format=json` output and return the
/// `disallowed_methods` diagnostics attributed to deny-list entries. A
/// diagnostic is attributed by the backtick-delimited configured path the
/// lint names in its message, so `Receiver::recv_timeout` never counts as
/// `Receiver::recv`.
///
/// A diagnostic is charged to a crate ONLY when some span of it renders the
/// call the lint names. Two classes fail that test and are returned with
/// `attributed: false`:
///
/// * macro-generated calls - `#[tokio::test]` expands to a runtime
///   `block_on`, and the generated span points at the test body's last
///   statement, so the file/line pair names an `assert_eq!` rather than a
///   call site. Counting it charges the crate for a call no author wrote.
/// * diagnostics whose spans name a file outside the crate's own sources.
///
/// Both are reported (so the number is visible) and neither is charged to a
/// crate's ratchet: an unattributable diagnostic must not be able to move a
/// committed baseline line.
pub fn parse_clippy_disallowed(json: &str, entries: &[DeniedApi]) -> Vec<ClippyHit> {
    let mut hits = Vec::new();
    for line in json.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        if message.get("code").and_then(|c| c.get("code")).and_then(|c| c.as_str())
            != Some("clippy::disallowed_methods")
        {
            continue;
        }
        let Some(text) = message.get("message").and_then(|m| m.as_str()) else {
            continue;
        };
        let Some(entry) = entries
            .iter()
            .find(|entry| text.contains(&format!("`{}`", entry.path)))
        else {
            continue;
        };
        // Attribute to the span that actually renders the call, not simply
        // to `spans.first()`: a macro-generated diagnostic carries the
        // expansion's span there, which names the token the macro spanned
        // over rather than the call.
        let spans = message
            .get("spans")
            .and_then(|s| s.as_array())
            .cloned()
            .unwrap_or_default();
        // Prefer a span that both renders the call and was not produced by a
        // macro expansion; fall back to any span that renders the call, so a
        // hand-written call is still found when rustc reports the expansion
        // alongside it.
        let call_span = spans
            .iter()
            .find(|span| span_contains_call(span, entry) && !span_is_macro_generated(span))
            .or_else(|| spans.iter().find(|span| span_contains_call(span, entry)));
        let primary = spans.iter().find(|span| {
            span.get("is_primary")
                .and_then(|p| p.as_bool())
                .unwrap_or(false)
        });
        let chosen = call_span.or(primary).or_else(|| spans.first());
        let file = chosen
            .and_then(|s| s.get("file_name"))
            .and_then(|f| f.as_str())
            .unwrap_or_default()
            .to_owned();
        let line_no = chosen
            .and_then(|s| s.get("line_start"))
            .and_then(|l| l.as_u64())
            .unwrap_or(0) as usize;
        // Attributable means: some span of this diagnostic renders the named
        // call at a non-expansion site. `#[tokio::test]`'s generated
        // `block_on` does not qualify - its span points at the test body's
        // last statement - so it is reported but not charged.
        let attributed = call_span.is_some()
            && !call_span.is_some_and(span_is_macro_generated);
        hits.push(ClippyHit {
            entry_path: entry.path.clone(),
            file,
            line: line_no,
            attributed,
        });
    }
    hits
}

/// One `#[allow]`/`#[expect]` suppression of a banned-API lint found in
/// source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuppressionSite {
    /// Repository-relative file path.
    pub file: String,
    /// 1-based line of the attribute's start.
    pub line: usize,
    /// `#![allow(...)]` / `#![expect(...)]` module-level blanket attribute.
    pub blanket: bool,
    /// `#[expect(...)]` rather than `#[allow(...)]`.
    pub expect: bool,
    /// The banned lint, e.g. `clippy::disallowed_methods`.
    pub lint: String,
    /// The attribute's `reason = "..."` value, when present.
    pub reason: Option<String>,
}

/// Scan one source file for `#[allow]`/`#[expect]` suppressions of
/// banned-API lints. Comments, strings, and char literals are skipped, so a
/// doc comment mentioning an allow never inventories one.
pub fn scan_suppressions(file: &str, text: &str) -> Vec<SuppressionSite> {
    let mut sites = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut line = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            b'"' => {
                let (next, newlines) = skip_quoted(bytes, i);
                line += newlines;
                i = next;
            }
            b'\'' if is_char_literal(bytes, i) => {
                let (next, newlines) = skip_quoted(bytes, i);
                line += newlines;
                i = next;
            }
            b'r' if matches!(bytes.get(i + 1), Some(b'#') | Some(b'"')) => {
                let (next, newlines) = skip_raw(bytes, i);
                line += newlines;
                i = next;
            }
            b'#' if matches!(bytes.get(i + 1), Some(b'[') | Some(b'!')) => {
                let attr_line = line;
                let start = i;
                let (next, newlines) = skip_attribute(bytes, i);
                line += newlines;
                i = next;
                let attr = String::from_utf8_lossy(&bytes[start..i]).into_owned();
                sites.extend(parse_suppression_attribute(file, attr_line, &attr));
            }
            _ => i += 1,
        }
    }
    sites
}

/// Skip a `"..."` or `'...'` literal starting at `i`, returning the index
/// after it and the newlines it spans.
fn skip_quoted(bytes: &[u8], mut i: usize) -> (usize, usize) {
    let quote = bytes[i];
    let mut newlines = 0;
    i += 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'\n' => {
                newlines += 1;
                i += 1;
            }
            c if c == quote => {
                i += 1;
                break;
            }
            _ => i += 1,
        }
    }
    (i, newlines)
}

/// Skip a `r#"..."#` raw string starting at the `r`, returning the index
/// after it and the newlines it spans.
fn skip_raw(bytes: &[u8], i: usize) -> (usize, usize) {
    let mut hashes = 0;
    let mut j = i + 1;
    while j < bytes.len() && bytes[j] == b'#' {
        hashes += 1;
        j += 1;
    }
    if j >= bytes.len() || bytes[j] != b'"' {
        return (i + 1, 0);
    }
    j += 1;
    let mut newlines = 0;
    while j < bytes.len() {
        if bytes[j] == b'\n' {
            newlines += 1;
            j += 1;
        } else if bytes[j] == b'"'
            && j + 1 + hashes <= bytes.len()
            && bytes[j + 1..j + 1 + hashes].iter().all(|b| *b == b'#')
        {
            j += 1 + hashes;
            break;
        } else {
            j += 1;
        }
    }
    (j, newlines)
}

/// Whether a `'` at `i` opens a char literal rather than a lifetime: a
/// closing quote follows on the same line, possibly through an escape.
fn is_char_literal(bytes: &[u8], i: usize) -> bool {
    let Some(&next) = bytes.get(i + 1) else {
        return false;
    };
    if next == b'\\' {
        return bytes.get(i + 3) == Some(&b'\'');
    }
    next != b'\'' && bytes.get(i + 2) == Some(&b'\'')
}

/// Skip a `#[...]` / `#![...]` attribute starting at the `#`, tracking
/// bracket depth and skipping strings, returning the index after the closing
/// `]` and the newlines it spans.
fn skip_attribute(bytes: &[u8], mut i: usize) -> (usize, usize) {
    let mut depth = 0usize;
    let mut newlines = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                newlines += 1;
                i += 1;
            }
            b'"' => {
                let (next, spanned) = skip_quoted(bytes, i);
                newlines += spanned;
                i = next;
            }
            b'\'' if is_char_literal(bytes, i) => {
                let (next, spanned) = skip_quoted(bytes, i);
                newlines += spanned;
                i = next;
            }
            b'r' if matches!(bytes.get(i + 1), Some(b'#') | Some(b'"')) => {
                let (next, spanned) = skip_raw(bytes, i);
                newlines += spanned;
                i = next;
            }
            b'[' => {
                depth += 1;
                i += 1;
            }
            b']' => {
                depth = depth.saturating_sub(1);
                i += 1;
                if depth == 0 {
                    break;
                }
            }
            _ => i += 1,
        }
    }
    (i, newlines)
}

/// Parse one collected attribute (`allow(...)`, `expect(...)`, or a
/// `cfg_attr` wrapping either) into the banned-API suppressions it carries.
fn parse_suppression_attribute(file: &str, line: usize, attr: &str) -> Vec<SuppressionSite> {
    let blanket = attr.starts_with("#![");
    let mut sites = Vec::new();
    let bytes = attr.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let (next, _) = skip_quoted(bytes, i);
                i = next;
            }
            b'a' | b'e' => {
                let keyword = if bytes[i..].starts_with(b"allow(") {
                    "allow"
                } else if bytes[i..].starts_with(b"expect(") {
                    "expect"
                } else {
                    i += 1;
                    continue;
                };
                let open = i + keyword.len();
                let (content_end, _) = skip_parens(bytes, open);
                let content = &attr[open + 1..content_end];
                let args = split_args(content);
                let expect = keyword == "expect";
                let reason = reason_in_args(&args);
                for lint in banned_lints_in_args(&args) {
                    sites.push(SuppressionSite {
                        file: file.to_owned(),
                        line,
                        blanket,
                        expect,
                        lint: lint.to_owned(),
                        reason: reason.clone(),
                    });
                }
                i = content_end + 1;
            }
            _ => i += 1,
        }
    }
    sites
}

/// Skip a balanced `(...)` group starting at `i` (which must be `(`),
/// returning the index of the closing `)`.
fn skip_parens(bytes: &[u8], mut i: usize) -> (usize, usize) {
    let mut depth = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let (next, _) = skip_quoted(bytes, i);
                i = next;
            }
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    (i, 0)
}

/// Split an argument list on top-level commas, keeping strings and nested
/// groups whole.
fn split_args(content: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    let mut chars = content.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        match c {
            '"' => {
                current.push(c);
                let mut escaped = false;
                for (_, sc) in chars.by_ref() {
                    current.push(sc);
                    if !escaped && sc == '"' {
                        break;
                    }
                    escaped = sc == '\\' && !escaped;
                }
            }
            '(' | '[' | '{' => {
                depth += 1;
                current.push(c);
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ',' if depth == 0 => {
                args.push(current.trim().to_owned());
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        args.push(current.trim().to_owned());
    }
    args
}

/// The banned-API lints an argument list names.
fn banned_lints_in_args(args: &[String]) -> Vec<&'static str> {
    BANNED_API_LINTS
        .iter()
        .copied()
        .filter(|lint| args.iter().any(|arg| arg.trim() == *lint))
        .collect()
}

/// The `reason = "..."` value an argument list carries, when present.
fn reason_in_args(args: &[String]) -> Option<String> {
    args.iter().find_map(|arg| {
        let rest = arg.trim().strip_prefix("reason")?;
        let rest = rest.trim_start().strip_prefix('=')?;
        let value = rest.trim();
        let value = value.strip_prefix('"')?.strip_suffix('"')?;
        Some(value.to_owned())
    })
}

/// One crate's census result: the authoritative per-entry counts and the
/// suppression inventory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrateCensus {
    /// Crate directory relative to the repo root, e.g. `packages/d2b-broker`.
    pub crate_dir: String,
    /// Package name from the manifest.
    pub package_name: String,
    /// Authoritative count per deny-list entry path.
    pub counts: BTreeMap<String, usize>,
    /// Module-level blanket suppressions of banned-API lints.
    pub blanket_suppressions: usize,
    /// Per-site suppressions of banned-API lints.
    pub per_site_suppressions: usize,
    /// Suppressions of the counting lint itself
    /// (`clippy::disallowed_methods`): the ratchet axis that keeps a count
    /// baseline from being lowered by silencing the lint that produces it.
    pub disallowed_method_suppressions: usize,
}

/// The committed per-crate baseline the CI cap compares against.
///
/// The map is keyed by crate directory (`packages/d2b-broker`) then by
/// deny-list entry path, so every deny-entry class - including
/// `tokio::task::spawn_blocking` - carries its own per-crate cap: the
/// no-new-spawn_blocking guard during the conversion window is the
/// spawn_blocking row, which the gate refuses to see grow.
///
/// `suppressions` is the second axis: crate directory to the number of
/// `#[allow(clippy::disallowed_methods)]` / `#[expect(...)]` sites the crate
/// carries. Both maps are enforced, so the count baseline can only be walked
/// down by deleting the call the count names, never by silencing the lint
/// that counts it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CensusBaseline {
    /// Crate-directory to deny-entry counts, as serialized into the
    /// committed baseline file.
    pub crates: BTreeMap<String, BTreeMap<String, usize>>,
    /// Crate-directory to `clippy::disallowed_methods` suppression sites. The
    /// map defaults to empty so a baseline written before this axis existed
    /// still parses, and then fails closed on the first crate that carries a
    /// suppression rather than passing vacuously. A crate the map does not
    /// mention is held to zero on both axes.
    #[serde(default)]
    pub suppressions: BTreeMap<String, usize>,
}

/// One source file under a census crate.
struct CensusFile {
    /// Repository-relative path.
    rel: String,
    split: SplitContext,
}

/// The workspace member paths the root manifest declares, e.g.
/// `packages/d2b-broker`. Shared with the async gate, which derives its
/// control-plane scan roots from the same member list.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub(crate) fn workspace_member_paths(repo_root: &Path) -> Result<BTreeSet<String>, String> {
    let manifest = fs::read_to_string(repo_root.join("Cargo.toml"))
        .map_err(|error| format!("blocking-census: read root Cargo.toml: {error}"))?;
    let mut members = BTreeSet::new();
    let mut in_members = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed == "members = [" {
            in_members = true;
            continue;
        }
        if !in_members {
            continue;
        }
        if trimmed == "]" {
            break;
        }
        let Some(relative) = trimmed.trim_end_matches(',').strip_prefix('"') else {
            continue;
        };
        let Some(relative) = relative.strip_suffix('"') else {
            continue;
        };
        members.insert(relative.to_owned());
    }
    Ok(members)
}

/// Resolve the census scope: the given crate paths, or every workspace
/// member crate (a member directory with a `Cargo.toml`) under `packages/`.
/// Non-member directories under `packages/` are not censused: they are not
/// workspace crates, so the workspace-wide clippy run cannot measure their
/// instance-method classes.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn resolve_crate_dirs(repo_root: &Path, crate_args: &[String]) -> Result<Vec<PathBuf>, String> {
    if crate_args.is_empty() {
        let members = workspace_member_paths(repo_root)?;
        let mut dirs = Vec::new();
        let entries = fs::read_dir(repo_root.join("packages"))
            .map_err(|error| format!("blocking-census: read packages/: {error}"))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("blocking-census: dir entry: {error}"))?;
            let path = entry.path();
            let relative = format!(
                "packages/{}",
                path.file_name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_default()
            );
            if path.is_dir() && path.join("Cargo.toml").is_file() && members.contains(&relative) {
                dirs.push(path);
            }
        }
        dirs.sort();
        return Ok(dirs);
    }
    let mut dirs = Vec::new();
    for arg in crate_args {
        let path = repo_root.join(arg);
        if !path.is_dir() {
            return Err(format!("blocking-census: not a crate directory: {arg}"));
        }
        if !path.join("Cargo.toml").is_file() {
            return Err(format!("blocking-census: no Cargo.toml in {arg}"));
        }
        dirs.push(path);
    }
    Ok(dirs)
}

/// The package name a crate directory's manifest declares. Shared with the
/// async gate, which resolves the same member crates' names to walk the
/// control-plane link graph.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub(crate) fn package_name(crate_dir: &Path) -> Result<String, String> {
    let manifest = fs::read_to_string(crate_dir.join("Cargo.toml"))
        .map_err(|error| format!("blocking-census: read {}: {error}", crate_dir.display()))?;
    manifest
        .lines()
        .find_map(|line| line.trim().strip_prefix("name = \""))
        .and_then(|name| name.strip_suffix('"'))
        .map(str::to_owned)
        .ok_or_else(|| format!("blocking-census: no package name in {}", crate_dir.display()))
}

/// Build the census clippy command: `cargo clippy --locked
/// --message-format=json --all-targets` over the given packages (or the
/// whole workspace) with the manifest's documented de-escalation set after
/// `--` (Cargo.toml, "Keep the census visible:"). `RUSTFLAGS` is cleared so
/// the `.cargo/config.toml` `-D warnings` rustflags cannot turn unrelated
/// warnings into failures, and `-W` (not `--force-warn`) is used so
/// `#[allow]` attributes keep suppressing - the same predicate the lints
/// enforce under deny. The de-escalation set is the contract: a stray
/// `await_holding_lock`/`await_holding_refcell_ref`/`disallowed_methods`
/// diagnostic must count, not fail the run.
fn clippy_command(repo_root: &Path, packages: &[String]) -> Command {
    let mut command = Command::new("cargo");
    command
        .arg("clippy")
        .arg("--locked")
        .arg("--message-format=json")
        .arg("--all-targets")
        .current_dir(repo_root)
        .env("RUSTFLAGS", "")
        .env("CARGO_TERM_COLOR", "never");
    if packages.is_empty() {
        command.arg("--workspace");
    } else {
        for package in packages {
            command.arg("-p").arg(package);
        }
    }
    command
        .arg("--")
        .arg("-W")
        .arg("warnings")
        .arg("-W")
        .arg("clippy::disallowed_methods")
        .arg("-W")
        .arg("clippy::await_holding_lock")
        .arg("-W")
        .arg("clippy::await_holding_refcell_ref");
    command
}

/// Find the first real rustc/clippy diagnostic in `stderr`: the first
/// `error`/`warning` header that carries a `--> file:line` span (the header
/// is followed by the span, possibly through message continuation lines).
/// Errors are preferred over warnings - the run only fails on errors, so a
/// leading warning would not be the cause. Returns `file:line: message`;
/// when no diagnostic carries a span the fallback is error-first: the first
/// error header's message, then a spanning warning, then the first header's
/// message, and `None` when stderr has no diagnostic at all.
fn first_diagnostic(stderr: &str) -> Option<String> {
    let lines: Vec<&str> = stderr.lines().collect();
    let mut first_warning: Option<String> = None;
    let mut first_spanless_error: Option<String> = None;
    let mut first_spanless: Option<String> = None;
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        let is_error = trimmed.starts_with("error");
        let is_warning = trimmed.starts_with("warning");
        if !is_error && !is_warning {
            i += 1;
            continue;
        }
        let Some(colon) = trimmed.find(": ") else {
            i += 1;
            continue;
        };
        let text = trimmed[colon + 2..].trim();
        if first_spanless_error.is_none() && is_error {
            first_spanless_error = Some(text.to_string());
        }
        if first_spanless.is_none() {
            first_spanless = Some(text.to_string());
        }
        // The span line follows the header, possibly after message
        // continuation lines; a following header, note, or help ends the
        // header's block.
        let mut span: Option<String> = None;
        for next in lines.iter().skip(i + 1).take(6) {
            let next_trimmed = next.trim_start();
            if let Some(rest) = next_trimmed.strip_prefix("--> ") {
                span = Some(rest.to_string());
                break;
            }
            if next_trimmed.starts_with("error")
                || next_trimmed.starts_with("warning")
                || next_trimmed.starts_with("note")
                || next_trimmed.starts_with("help")
                || next_trimmed.starts_with("= ")
            {
                break;
            }
        }
        if let Some(rest) = span {
            let loc = rest.split(':').take(2).collect::<Vec<_>>().join(":");
            let diagnostic = format!("{loc}: {text}");
            if is_error {
                return Some(diagnostic);
            }
            if first_warning.is_none() {
                first_warning = Some(diagnostic);
            }
        }
        i += 1;
    }
    first_spanless_error.or(first_warning).or(first_spanless)
}

/// Find the first real diagnostic in the `--message-format=json` stream:
/// the first `compiler-message` at error level (falling back to warning)
/// that carries a primary span, returned as `file:line: message`. Errors
/// are preferred over warnings - the run only fails on errors, so a leading
/// warning would not be the cause. When no error-level record carries a
/// primary span, the first error-level record's bare message is returned
/// before any warning is considered; `None` when the stream has no
/// error-level record and no warning that carries a primary span (a
/// spanless warning is dropped, so a spanless-warning-only stream also
/// yields `None` - e.g. a cargo-level failure that never reached rustc).
fn first_json_diagnostic(json: &str) -> Option<String> {
    let mut first_warning: Option<String> = None;
    let mut first_spanless_error: Option<String> = None;
    for line in json.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        let Some(level) = message.get("level").and_then(|l| l.as_str()) else {
            continue;
        };
        let Some(text) = message.get("message").and_then(|m| m.as_str()) else {
            continue;
        };
        let spanned = (|| {
            let primary = message
                .get("spans")
                .and_then(|s| s.as_array())
                .and_then(|spans| {
                    spans
                        .iter()
                        .find(|span| span.get("is_primary").and_then(|p| p.as_bool()) == Some(true))
                })?;
            let file = primary.get("file_name").and_then(|f| f.as_str())?;
            let line_start = primary.get("line_start").and_then(|l| l.as_u64())?;
            Some(format!("{file}:{line_start}: {text}"))
        })();
        if level == "error" {
            if let Some(diagnostic) = spanned {
                return Some(diagnostic);
            }
            if first_spanless_error.is_none() {
                first_spanless_error = Some(text.to_string());
            }
            continue;
        }
        if first_warning.is_none() && let Some(diagnostic) = spanned {
            first_warning = Some(diagnostic);
        }
    }
    first_spanless_error.or(first_warning)
}

// ---- Error-level-only picks -------------------------------------
/// The first error-level header in the stderr text, span or spanless,
/// returned as `file:line: message` when it carries a span and the bare
/// message otherwise. The text parser already ranks errors ahead of
/// warnings, so this just discards the warning-level fallback - the caller
/// (run_clippy) consults it before accepting a warning-only JSON pick.
fn first_stderr_error(stderr: &str) -> Option<String> {
    // First error-level header, span or spanless: cargo-level failures
    // (e.g. `error: failed to run custom build command`) never carry a
    // `--> file:line` span, but they are the real cause when the JSON
    // stream holds only warnings.
    let lines: Vec<&str> = stderr.lines().collect();
    let mut first_error_spanless: Option<String> = None;
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if !trimmed.starts_with("error") {
            i += 1;
            continue;
        }
        let Some(colon) = trimmed.find(": ") else {
            i += 1;
            continue;
        };
        let text = trimmed[colon + 2..].trim();
        if first_error_spanless.is_none() {
            first_error_spanless = Some(text.to_string());
        }
        // The span line follows the header, possibly after message
        // continuation lines; a following header, note, or help ends the
        // header's block.
        for next in lines.iter().skip(i + 1).take(6) {
            let next_trimmed = next.trim_start();
            if let Some(rest) = next_trimmed.strip_prefix("--> ") {
                let loc = rest.split(':').take(2).collect::<Vec<_>>().join(":");
                return Some(format!("{loc}: {text}"));
            }
            if next_trimmed.starts_with("error")
                || next_trimmed.starts_with("warning")
                || next_trimmed.starts_with("note")
                || next_trimmed.starts_with("help")
                || next_trimmed.starts_with("= ")
            {
                break;
            }
        }
        i += 1;
    }
    first_error_spanless
}
/// The first error-level record in the `--message-format=json` stream,
/// span or spanless. This is the pick the census run's failure message is
/// built from: errors are ranked ahead of any warning because the run only
/// fails on errors, and a spanless cargo-level error outranks a warning
/// that carried a primary span.
fn first_json_error(json: &str) -> Option<String> {
    let mut first_spanless_error: Option<String> = None;
    for line in json.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        let Some(level) = message.get("level").and_then(|l| l.as_str()) else {
            continue;
        };
        if level != "error" {
            continue;
        }
        let Some(text) = message.get("message").and_then(|m| m.as_str()) else {
            continue;
        };
        let spanned = (|| {
            let primary = message
                .get("spans")
                .and_then(|s| s.as_array())
                .and_then(|spans| {
                    spans
                        .iter()
                        .find(|span| span.get("is_primary").and_then(|p| p.as_bool()) == Some(true))
                })?;
            let file = primary.get("file_name").and_then(|f| f.as_str())?;
            let line_start = primary.get("line_start").and_then(|l| l.as_u64())?;
            Some(format!("{file}:{line_start}: {text}"))
        })();
        if let Some(diagnostic) = spanned {
            return Some(diagnostic);
        }
        if first_spanless_error.is_none() {
            first_spanless_error = Some(text.to_string());
        }
    }
    first_spanless_error
}
/// Run the census clippy command over the given packages (or the whole
/// workspace) and return the JSON stream. On failure the error surfaces the
/// first real diagnostic (file:line and message) rather than a reversed
/// tail, so a compile error in a large crate stays actionable. Under
/// `--message-format=json` the diagnostics live in the JSON stream (stderr
/// only carries cargo's own messages), so the JSON stream is parsed first;
/// the stderr text is the fallback for cargo-level failures, and the
/// reversed tail only survives as the last resort.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn run_clippy(repo_root: &Path, packages: &[String]) -> Result<String, String> {
    let output = clippy_command(repo_root, packages)
        .output()
        .map_err(|error| format!("blocking-census: cargo clippy launch failed: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let diagnostic = first_json_error(&stdout)
            .or_else(|| first_stderr_error(&stderr))
            .or_else(|| first_json_diagnostic(&stdout))
            .or_else(|| first_diagnostic(&stderr))
            .unwrap_or_else(|| {
                let tail: Vec<&str> = stderr.lines().rev().take(15).collect();
                tail.join("
")
            });
        return Err(format!(
            "blocking-census: cargo clippy failed (exit {}):\n{diagnostic}",
            output.status
        ));
    }
    Ok(stdout)
}

/// Walk a directory tree for `.rs` files, skipping `target/` directories.
pub fn walk_rs(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    walk(root, &mut found)?;
    Ok(found)
}

#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn walk(dir: &Path, found: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|error| format!("blocking-census: read dir {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("blocking-census: dir entry: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            walk(&path, found)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
    Ok(())
}

/// Collect one crate's census inputs: every `.rs` file with its prod/test
/// split and suppression inventory. `disallowed_method` accumulates the
/// suppression sites of the counting lint itself, the axis that keeps the
/// count baseline from being lowered by silencing the lint.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn collect_crate_files(
    repo_root: &Path,
    crate_dir: &Path,
    files: &mut Vec<CensusFile>,
    blanket: &mut usize,
    per_site: &mut usize,
    disallowed_method: &mut usize,
) -> Result<(), String> {
    for path in walk_rs(crate_dir)? {
        let rel = path
            .strip_prefix(repo_root)
            .map_err(|_| format!("blocking-census: {} outside repo root", path.display()))?
            .to_string_lossy()
            .into_owned();
        let raw = fs::read_to_string(&path)
            .map_err(|error| format!("blocking-census: read {}: {error}", path.display()))?;
        let split = split_contexts(&raw, is_test_dir(&path));
        for site in scan_suppressions(&rel, &raw) {
            if site.lint == COUNTING_LINT {
                *disallowed_method += 1;
            }
            if site.blanket {
                *blanket += 1;
            } else {
                *per_site += 1;
            }
        }
        files.push(CensusFile { rel, split });
    }
    Ok(())
}

/// Whether a clippy hit at `file:line` is test context, by the same split
/// the lexical meter uses.
fn hit_is_test(files: &[CensusFile], file: &str, line: usize) -> bool {
    files
        .iter()
        .find(|census_file| census_file.rel == file)
        .is_some_and(|census_file| {
            census_file
                .split
                .1
                .iter()
                .any(|(line_number, _)| *line_number == line)
        })
}

/// Fail when the run moved past the committed baseline on either axis.
///
/// Axis one is the per-entry deny-list count: no crate may exceed its
/// committed count for any entry class. Axis two is the per-crate count of
/// `clippy::disallowed_methods` suppressions: no crate may carry more of
/// them than the baseline records. Axis two is what stops the walk-down -
/// without it, one `#[allow]` outside a provider crate deletes the
/// diagnostic the count is derived from and lowers the committed baseline by
/// one while every gate stays green, until the count reaches zero.
///
/// A crate the baseline does not mention is held to zero on both axes, so a
/// new crate must ship a baseline entry the moment it carries a blocking call
/// or a suppression.
fn check_against_baseline(baseline_path: &Path, crates: &[CrateCensus]) -> Result<(), String> {
    let committed = fs::read_to_string(baseline_path)
        .map_err(|error| format!("blocking-census: read baseline {}: {error}", baseline_path.display()))?;
    let committed: CensusBaseline = serde_json::from_str(&committed)
        .map_err(|error| format!("blocking-census: parse baseline {}: {error}", baseline_path.display()))?;
    let mut violations = Vec::new();
    for census in crates {
        let committed_counts = committed.crates.get(&census.crate_dir);
        let mut crate_violations = Vec::new();
        for (entry, count) in &census.counts {
            let committed_count = committed_counts
                .and_then(|counts| counts.get(entry))
                .copied()
                .unwrap_or(0);
            if *count > committed_count {
                crate_violations.push(format!(
                    "{entry}: {count} > {committed_count} (committed baseline)"
                ));
            }
        }
        let committed_suppressions = committed
            .suppressions
            .get(&census.crate_dir)
            .copied()
            .unwrap_or(0);
        if census.disallowed_method_suppressions > committed_suppressions {
            crate_violations.push(format!(
                "{COUNTING_LINT} suppressions: {} > {committed_suppressions} (committed baseline): \
                 a new allow/expect lowers the blocking-call baseline instead of removing a call",
                census.disallowed_method_suppressions
            ));
        }
        if crate_violations.is_empty() {
            println!(
                "  {}: {} entry class(es) at or below baseline, {} suppression(s) at or below baseline",
                census.crate_dir,
                census.counts.len(),
                census.disallowed_method_suppressions
            );
        } else {
            println!("  {}: ABOVE BASELINE", census.crate_dir);
            for violation in &crate_violations {
                println!("    {violation}");
            }
            violations.extend(crate_violations.iter().map(|violation| {
                format!("{}: {violation}", census.crate_dir)
            }));
        }
    }
    if violations.is_empty() {
        println!(
            "blocking-census check: PASS (no crate above its committed counts or suppression baseline)"
        );
    } else {
        return Err(format!(
            "blocking-census check: FAILED - {} baseline item(s) above the committed baseline {}:\n{}",
            violations.len(),
            baseline_path.display(),
            violations.join("\n")
        ));
    }
    Ok(())
}

/// Judge the run against the committed baseline FIRST, then write the
/// regenerated file.
///
/// The order is the contract, not a convenience: `--json <committed>
/// --check <committed>` is the natural way to re-baseline in place, and
/// writing first would let the run compare the tree against the baseline it
/// just measured and pass unconditionally. Checking first also leaves the
/// committed file untouched when the run is over it.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn check_then_write_baseline(
    baseline: Option<&Path>,
    json_out: Option<&Path>,
    crates: &[CrateCensus],
) -> Result<(), String> {
    if let Some(baseline_path) = baseline {
        check_against_baseline(baseline_path, crates)?;
    }
    let Some(path) = json_out else {
        return Ok(());
    };
    let baseline_record = CensusBaseline {
        crates: crates
            .iter()
            .map(|census| (census.crate_dir.clone(), census.counts.clone()))
            .collect(),
        suppressions: crates
            .iter()
            .map(|census| {
                (
                    census.crate_dir.clone(),
                    census.disallowed_method_suppressions,
                )
            })
            .collect(),
    };
    let rendered = serde_json::to_string_pretty(&baseline_record)
        .map_err(|error| format!("blocking-census: serialize baseline: {error}"))?;
    fs::write(path, rendered + "\n")
        .map_err(|error| format!("blocking-census: write {}: {error}", path.display()))?;
    println!("baseline written: {}", path.display());
    Ok(())
}

/// Run the census over the repository or the given crate paths. Prints the
/// per-crate tables and the totals; with `baseline` fails when any covered
/// crate moved past its committed counts or its committed suppression count;
/// with `json_out` writes the regenerated baseline, always AFTER the check.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub fn run(
    repo_root: &Path,
    crate_args: &[String],
    json_out: Option<&Path>,
    baseline: Option<&Path>,
) -> Result<(), String> {
    let clippy_toml = fs::read_to_string(repo_root.join("clippy.toml"))
        .map_err(|_| "blocking-census: clippy.toml unreadable".to_owned())?;
    let entries = parse_deny_list(&clippy_toml);
    if entries.is_empty() {
        return Err("blocking-census: no entries parsed from clippy.toml".to_owned());
    }

    let crate_dirs = resolve_crate_dirs(repo_root, crate_args)?;
    let package_names: Vec<String> = crate_dirs
        .iter()
        .map(|dir| package_name(dir))
        .collect::<Result<_, _>>()?;

    // Workspace-wide runs use `--workspace` (every workspace member);
    // per-crate runs select exactly the requested packages.
    let clippy_json = if crate_args.is_empty() {
        run_clippy(repo_root, &[])?
    } else {
        run_clippy(repo_root, &package_names)?
    };
    let hits = parse_clippy_disallowed(&clippy_json, &entries);

    let mut crates: Vec<CrateCensus> = Vec::new();
    let mut production_total = 0usize;
    let mut test_total = 0usize;
    let mut clippy_total = 0usize;
    let mut authoritative_total = 0usize;

    for (crate_dir, package_name) in crate_dirs.iter().zip(&package_names) {
        let rel_dir = crate_dir
            .strip_prefix(repo_root)
            .map_err(|_| "blocking-census: crate dir outside repo root".to_owned())?
            .to_string_lossy()
            .into_owned();
        let prefix = format!("{rel_dir}/");

        let mut files = Vec::new();
        let mut blanket = 0usize;
        let mut per_site = 0usize;
        let mut disallowed_method = 0usize;
        collect_crate_files(
            repo_root,
            crate_dir,
            &mut files,
            &mut blanket,
            &mut per_site,
            &mut disallowed_method,
        )?;

        let text_counts = count_occurrences(
            &files
                .iter()
                .map(|file| (file.rel.clone(), file.split.clone()))
                .collect::<Vec<_>>(),
            &entries,
        );

        // One pass per entry: the lexical meter for textual classes, the
        // clippy diagnostics for instance-method classes, with the prod/test
        // split applied to the clippy hits by re-splitting their files.
        let mut rows = Vec::new();
        let mut counts = BTreeMap::new();
        for (entry, text_count) in entries.iter().zip(&text_counts) {
            // The crate's own prefix is required, so a diagnostic emitted
            // while compiling a dependency is never charged here.
            //
            // Attributability is required only for the instance-method
            // classes, which have no other meter. A macro-generated call -
            // `#[tokio::test]` expands to a runtime `block_on` - spans the
            // test body's last statement rather than a call site, so
            // charging it would bill each async test one phantom call and
            // move counts on code whose call sites never changed. The
            // textual classes keep counting every diagnostic: their call is
            // written at the span or reached through an import, and
            // filtering them by rendered span text would under-count real
            // bare-import calls.
            let needs_attribution = matches!(entry.kind, EntryKind::InstanceMethod);
            let clippy_count = hits
                .iter()
                .filter(|hit| {
                    (!needs_attribution || hit.attributed)
                        && hit.entry_path == entry.path
                        && hit.file.starts_with(&prefix)
                })
                .count();
            let (production, test, source) = match entry.kind {
                EntryKind::Textual => (text_count.production, text_count.test, "lexical"),
                EntryKind::InstanceMethod => {
                    let in_production = hits
                        .iter()
                        .filter(|hit| {
                            hit.attributed
                                && hit.entry_path == entry.path
                                && hit.file.starts_with(&prefix)
                                && !hit_is_test(&files, &hit.file, hit.line)
                        })
                        .count();
                    (in_production, clippy_count - in_production, "clippy")
                }
            };
            // Clippy deny is the flip authority (plan R10), so the
            // clippy-derived diagnostic count is the authoritative count for
            // every entry class: the lexical meter cannot see instance
            // methods, and overcounts doc-comment mentions and undercounts
            // imported bare calls for free functions - the lint measures the
            // actual calls. The lexical columns stay in the report as the
            // fixed meter (full configured path text, so tokio-replacement
            // lines never count).
            let authoritative = clippy_count;
            production_total += production;
            test_total += test;
            clippy_total += clippy_count;
            authoritative_total += authoritative;
            counts.insert(entry.path.clone(), authoritative);
            rows.push((entry, text_count, production, test, clippy_count, source));
        }

        println!("\n== {rel_dir} ({package_name}) ==");
        println!(
            "{:72} {:>4} {:>4} {:>6}  source",
            "denied API (clippy.toml path)", "prod", "test", "clippy"
        );
        let mut reported = 0;
        for (entry, text_count, production, test, clippy_count, source) in &rows {
            if *production > 0 || *test > 0 || *clippy_count > 0 {
                reported += 1;
                println!(
                    "{:72} {:>4} {:>4} {:>6}  {source}",
                    entry.path, production, test, clippy_count
                );
                for sample in &text_count.samples {
                    println!("  {:72}  {}", "", sample);
                }
            }
        }
        if reported == 0 {
            println!("{:72} {:>4} {:>4} {:>6}  -", "(no uses of any denied API)", 0, 0, 0);
        }
        println!(
            "suppressions: {blanket} blanket allow(s), {per_site} per-site allow(s)/expect(s) of banned-API lints"
        );
        println!(
            "  of which {disallowed_method} suppress {COUNTING_LINT}, the lint the counts come from"
        );

        crates.push(CrateCensus {
            crate_dir: rel_dir,
            package_name: package_name.clone(),
            counts,
            blanket_suppressions: blanket,
            per_site_suppressions: per_site,
            disallowed_method_suppressions: disallowed_method,
        });
    }

    println!();
    println!("crates censused: {}", crates.len());
    println!("production blocking-API call sites: {production_total}");
    println!("test-context blocking-API call sites: {test_total}");
    println!("clippy-visible blocking-API call sites: {clippy_total}");
    println!("authoritative blocking-API call sites: {authoritative_total}");
    println!("deny-list entries: {}", entries.len());
    check_then_write_baseline(baseline, json_out, &crates)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway directory, removed on drop, for the baseline files these
    /// ratchet tests read and write.
    struct TempBaseline {
        dir: PathBuf,
    }

    static NEXT_TEMP_BASELINE: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl TempBaseline {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "blocking-census-baseline-test-{}-{}",
                std::process::id(),
                NEXT_TEMP_BASELINE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create temp dir");
            Self { dir }
        }

        fn path(&self) -> PathBuf {
            self.dir.join("baseline.json")
        }

        fn write_baseline(&self, baseline: &CensusBaseline) -> String {
            let rendered = format!(
                "{}\n",
                serde_json::to_string_pretty(baseline).expect("serialize the fixture baseline")
            );
            fs::write(self.path(), &rendered).expect("write the fixture baseline");
            rendered
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl Drop for TempBaseline {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    const FIXTURE_CRATE: &str = "packages/d2b-broker";
    const FIXTURE_ENTRY: &str = "std::fs::read";

    fn fixture_census(count: usize, suppressions: usize) -> CrateCensus {
        CrateCensus {
            crate_dir: FIXTURE_CRATE.to_owned(),
            package_name: "d2b-broker".to_owned(),
            counts: BTreeMap::from([(FIXTURE_ENTRY.to_owned(), count)]),
            blanket_suppressions: suppressions,
            per_site_suppressions: 0,
            disallowed_method_suppressions: suppressions,
        }
    }

    fn fixture_baseline(count: usize, suppressions: usize) -> CensusBaseline {
        CensusBaseline {
            crates: BTreeMap::from([(
                FIXTURE_CRATE.to_owned(),
                BTreeMap::from([(FIXTURE_ENTRY.to_owned(), count)]),
            )]),
            suppressions: BTreeMap::from([(FIXTURE_CRATE.to_owned(), suppressions)]),
        }
    }

    #[test]
    fn baseline_check_fails_when_a_count_grows_above_the_committed_baseline() {
        // A committed baseline of zero is the common row: one new blocking
        // call is one over the line.
        let temp = TempBaseline::new();
        temp.write_baseline(&fixture_baseline(0, 0));
        let error = check_against_baseline(&temp.path(), &[fixture_census(1, 0)])
            .expect_err("one new blocking call over a zero baseline must fail");
        assert!(error.contains("std::fs::read: 1 > 0"), "{error}");
        assert!(error.contains(FIXTURE_CRATE), "{error}");
    }

    #[test]
    fn baseline_check_fails_when_a_new_suppression_lowers_a_count() {
        // The walk-down: an `#[allow(clippy::disallowed_methods)]` outside a
        // provider crate deletes the diagnostic the count comes from, so the
        // count drops from 1 to 0 and the committed baseline would come down
        // with it. With the suppression axis attached, the drop is the
        // evidence of the trick, not the proof of progress.
        let temp = TempBaseline::new();
        temp.write_baseline(&fixture_baseline(1, 0));
        let error = check_against_baseline(&temp.path(), &[fixture_census(0, 1)])
            .expect_err("a lowered count backed by a new allow must fail");
        assert!(
            error.contains("suppressions: 1 > 0"),
            "the failure must name the suppression that hid the call: {error}"
        );
        assert!(error.contains(COUNTING_LINT), "{error}");
    }

    #[test]
    fn baseline_check_passes_when_the_call_is_actually_removed() {
        // The legitimate walk-down stays legal: the same count drop with no
        // new suppression is a conversion, which is the point of the
        // ratchet.
        let temp = TempBaseline::new();
        temp.write_baseline(&fixture_baseline(1, 0));
        check_against_baseline(&temp.path(), &[fixture_census(0, 0)])
            .expect("a removed call must pass the ratchet");
    }

    #[test]
    fn baseline_check_fails_closed_when_the_suppression_axis_is_missing() {
        // A baseline written before the suppression axis existed must not
        // pass vacuously: it parses as no suppressions anywhere, so the
        // fixture's committed allow is over its line.
        let temp = TempBaseline::new();
        fs::write(
            temp.path(),
            "{\n  \"crates\": {\n    \"packages/d2b-broker\": {\n      \"std::fs::read\": 1\n    }\n  }\n}\n",
        )
        .expect("write the axis-less baseline");
        let error = check_against_baseline(&temp.path(), &[fixture_census(1, 1)])
            .expect_err("an axis-less baseline must not pass");
        assert!(error.contains("suppressions: 1 > 0"), "{error}");
    }

    #[test]
    fn regeneration_is_judged_against_the_committed_baseline_before_it_overwrites_it() {
        // `--json <committed> --check <committed>` is the natural in-place
        // re-baseline. Writing first would let the run compare the tree
        // against the baseline it just measured, so an added allow would
        // lower the committed file and pass unconditionally. The check runs
        // first, and a failed check leaves the committed bytes alone.
        let temp = TempBaseline::new();
        let committed = temp.write_baseline(&fixture_baseline(1, 0));
        let error = check_then_write_baseline(
            Some(&temp.path()),
            Some(&temp.path()),
            &[fixture_census(0, 1)],
        )
        .expect_err("the in-place re-baseline must be judged against the committed bytes");
        assert!(error.contains("suppressions: 1 > 0"), "{error}");
        let after = fs::read_to_string(temp.path()).expect("read the baseline back");
        assert_eq!(
            after,
            committed,
            "a refused re-baseline must leave the committed file untouched"
        );
    }

    #[test]
    fn deny_list_parses_the_committed_list() {
        let toml = include_str!("../../../clippy.toml");
        let entries = parse_deny_list(toml);
        assert!(entries.len() >= 54, "deny list shrank: {}", entries.len());
        assert!(entries.iter().any(|entry| entry.tail == "thread::sleep"));
        assert!(entries.iter().any(|entry| entry.path == "std::sync::Mutex::lock"));
    }

    #[test]
    fn instance_method_entries_are_classified_for_clippy_counting() {
        let toml = include_str!("../../../clippy.toml");
        let entries = parse_deny_list(toml);
        let by_path: BTreeMap<&str, EntryKind> = entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.kind))
            .collect();
        assert_eq!(by_path["std::sync::Mutex::lock"], EntryKind::InstanceMethod);
        assert_eq!(by_path["std::sync::mpsc::Receiver::recv"], EntryKind::InstanceMethod);
        assert_eq!(by_path["std::fs::File::open"], EntryKind::InstanceMethod);
        assert_eq!(by_path["std::thread::JoinHandle::join"], EntryKind::InstanceMethod);
        assert_eq!(by_path["std::fs::read_to_string"], EntryKind::Textual);
        assert_eq!(by_path["std::thread::sleep"], EntryKind::Textual);
        assert_eq!(by_path["tokio::task::spawn_blocking"], EntryKind::Textual);
        assert_eq!(by_path["nix::sys::socket::recv"], EntryKind::Textual);
    }

    #[test]
    fn cfg_test_lines_count_as_test_context() {
        let raw = "\
use std::thread;

pub fn run() {
    thread::sleep(std::time::Duration::from_secs(1));
}

#[cfg(test)]
mod tests {
    #[test]
    fn sleeps() {
        thread::sleep(std::time::Duration::from_secs(1));
    }
}

pub fn after() {}
";
        let (production, test) = split_contexts(raw, false);
        let production_has_sleep = production
            .iter()
            .any(|(_, text)| text.contains("thread::sleep"));
        let test_has_sleep = test.iter().any(|(_, text)| text.contains("thread::sleep"));
        assert!(production_has_sleep, "production call must stay production");
        assert!(test_has_sleep, "cfg(test) call must move to test context");
        assert!(production.iter().any(|(_, text)| text.contains("after()")));
    }

    #[test]
    fn tokio_replacement_lines_never_count_against_std_entries() {
        let entries = parse_deny_list(
            "disallowed-methods = [\n    { path = \"std::fs::read_to_string\", reason = \"x\", replacement = \"tokio\" },\n]\n",
        );
        let raw = "\
use std::fs;
use tokio::fs;

pub fn converted() {
    let _ = tokio::fs::read_to_string(\"/tmp/x\");
    let _ = std::fs::read_to_string(\"/tmp/y\");
}

pub fn not_converted() {
    let _ = std::fs::read_to_string(\"/tmp/z\");
}
";
        let files = vec![("fixture.rs".to_owned(), split_contexts(raw, false))];
        let counts = count_occurrences(&files, &entries);
        assert_eq!(counts[0].production, 2, "std::fs::read_to_string call sites");
        assert_eq!(counts[0].test, 0);
        assert!(!counts[0].samples.is_empty());
        let sample = &counts[0].samples[0];
        assert!(!sample.contains("tokio"), "tokio line must never be sampled");
    }

    #[test]
    fn full_path_matching_never_confuses_sibling_entries() {
        let entries = parse_deny_list(
            "disallowed-methods = [\n    { path = \"std::fs::read\", reason = \"x\", replacement = \"tokio\" },\n    { path = \"std::fs::read_to_string\", reason = \"x\", replacement = \"tokio\" },\n    { path = \"std::sync::mpsc::Receiver::recv\", reason = \"x\", replacement = \"tokio\" },\n    { path = \"std::sync::mpsc::Receiver::recv_timeout\", reason = \"x\", replacement = \"tokio\" },\n]\n",
        );
        let raw = "\
pub fn f() {
    let _ = std::fs::read(\"/tmp/a\");       // read, not read_to_string
    let _ = std::fs::read_to_string(\"/tmp/b\"); // read_to_string, not read
    let _ = std::fs::read_dir(\"/tmp/c\");   // read_dir, not read
    let _ = std::sync::mpsc::Receiver::recv(&rx);
    let _ = std::sync::mpsc::Receiver::recv_timeout(&rx, d);
}
";
        let files = vec![("fixture.rs".to_owned(), split_contexts(raw, false))];
        let counts = count_occurrences(&files, &entries);
        let by_path: BTreeMap<&str, &EntryCounts> = entries
            .iter()
            .zip(&counts)
            .map(|(entry, count)| (entry.path.as_str(), count))
            .collect();
        assert_eq!(by_path["std::fs::read"].production, 1);
        assert_eq!(by_path["std::fs::read_to_string"].production, 1);
        assert_eq!(by_path["std::sync::mpsc::Receiver::recv"].production, 1);
        assert_eq!(by_path["std::sync::mpsc::Receiver::recv_timeout"].production, 1);
    }

    #[test]
    fn clippy_json_attributes_hits_to_configured_paths() {
        let entries = parse_deny_list(
            "disallowed-methods = [\n    { path = \"std::sync::Mutex::lock\", reason = \"x\", replacement = \"tokio\" },\n    { path = \"std::sync::mpsc::Receiver::recv\", reason = \"x\", replacement = \"tokio\" },\n    { path = \"std::sync::mpsc::Receiver::recv_timeout\", reason = \"x\", replacement = \"tokio\" },\n    { path = \"std::io::Write::write_all\", reason = \"x\", replacement = \"tokio\" },\n]\n",
        );
        // Each span renders the call it names, which is what makes a hit
        // attributable: a span that does not show the call is not a site.
        let json = r#"
{"reason":"compiler-message","message":{"code":{"code":"clippy::disallowed_methods"},"message":"use of a disallowed method `std::sync::Mutex::lock`","spans":[{"file_name":"packages/d2b-broker/src/runtime.rs","line_start":41,"is_primary":true,"text":[{"text":"    state.lock()"}]}]}}
{"reason":"compiler-message","message":{"code":{"code":"clippy::disallowed_methods"},"message":"use of a disallowed method `std::sync::mpsc::Receiver::recv_timeout`","spans":[{"file_name":"packages/d2b-broker/src/runtime.rs","line_start":42,"is_primary":true,"text":[{"text":"    rx.recv_timeout(d)"}]}]}}
{"reason":"compiler-message","message":{"code":{"code":"clippy::disallowed_methods"},"message":"use of a disallowed method `std::io::Write::write_all`","spans":[{"file_name":"packages/d2b-broker/src/audit.rs","line_start":7,"is_primary":true,"text":[{"text":"    sink.write_all(&buf)"}]}]}}
{"reason":"compiler-message","message":{"code":{"code":"unused_variables"},"message":"unused variable"}}
{"reason":"build-finished","success":true}
"#;
        let hits = parse_clippy_disallowed(json, &entries);
        let recv = hits
            .iter()
            .filter(|hit| hit.entry_path == "std::sync::mpsc::Receiver::recv")
            .count();
        let recv_timeout = hits
            .iter()
            .filter(|hit| hit.entry_path == "std::sync::mpsc::Receiver::recv_timeout")
            .count();
        assert_eq!(recv, 0, "recv_timeout must never attribute to recv");
        assert!(
            hits.iter().all(|hit| hit.attributed),
            "a span that renders the named call is attributable: {hits:?}"
        );
        assert_eq!(recv_timeout, 1);
        assert_eq!(
            hits.iter()
                .filter(|hit| hit.entry_path == "std::sync::Mutex::lock")
                .count(),
            1
        );
        assert_eq!(
            hits.iter()
                .filter(|hit| hit.entry_path == "std::io::Write::write_all")
                .count(),
            1
        );
        assert_eq!(hits.len(), 3);
    }

    /// The meter bug this pins: `#[tokio::test]` expands to
    /// `Runtime::block_on(async { .. })` built with `quote_spanned!`, so the
    /// diagnostic's span points at the test body's LAST STATEMENT - an
    /// `assert_eq!` line - not at a call site. Taking `spans.first()` charged
    /// the crate one phantom `block_on` per async test, which is how d2bd's
    /// count moved 10 -> 39 on a file whose real call sites never changed.
    #[test]
    fn a_macro_generated_call_is_reported_but_not_charged() {
        let entries = parse_deny_list(
            "disallowed-methods = [\n    { path = \"tokio::runtime::Runtime::block_on\", reason = \"x\", replacement = \"async\" },\n]\n",
        );
        // The shape clippy actually emits for `#[tokio::test]`: one primary
        // span, an expansion block on it, and rendered text that is the
        // test's trailing `assert_eq!` - containing no `block_on` at all.
        let json = r#"
{"reason":"compiler-message","message":{"code":{"code":"clippy::disallowed_methods"},"message":"use of a disallowed method `tokio::runtime::Runtime::block_on`","spans":[{"file_name":"packages/d2bd/tests/authority_publication.rs","line_start":628,"column_start":47,"column_end":48,"is_primary":true,"expansion":{"span":{"file_name":"packages/d2bd/tests/authority_publication.rs","line_start":628}},"text":[{"highlight_start":47,"highlight_end":48,"text":"    assert_eq!(accepted.sequence, sequence(1));"}]}]}}
"#;
        let hits = parse_clippy_disallowed(json, &entries);
        assert_eq!(hits.len(), 1, "the diagnostic is still reported");
        assert!(
            !hits[0].attributed,
            "a macro-generated block_on spans an assert_eq!, so it must not be charged: {hits:?}"
        );
        assert!(
            span_is_macro_generated(
                &serde_json::from_str::<serde_json::Value>(
                    r#"{"expansion":{"span":{"line_start":628}}}"#
                )
                .expect("valid span json")
            ),
            "the expansion marker is what identifies the generated span"
        );
    }

    /// A diagnostic whose spans never render the named call is reported but
    /// not charged, so an unattributable hit cannot move a committed
    /// baseline line.
    #[test]
    fn a_span_that_does_not_render_the_call_is_not_attributable() {
        let entries = parse_deny_list(
            "disallowed-methods = [\n    { path = \"std::sync::Mutex::lock\", reason = \"x\", replacement = \"tokio\" },\n]\n",
        );
        let json = r#"
{"reason":"compiler-message","message":{"code":{"code":"clippy::disallowed_methods"},"message":"use of a disallowed method `std::sync::Mutex::lock`","spans":[{"file_name":"packages/d2b-broker/src/runtime.rs","line_start":9,"is_primary":true,"text":[{"text":"    let total = compute();"}]}]}}
"#;
        let hits = parse_clippy_disallowed(json, &entries);
        assert_eq!(hits.len(), 1);
        assert!(
            !hits[0].attributed,
            "a span rendering an unrelated call is not the denied site: {hits:?}"
        );
    }

    /// A textual entry reached through a bare import (`use std::fs;` then
    /// `read_to_string(...)`) has no full-path text at its span, so it is
    /// not attributable. Counting must NOT be gated on attributability for
    /// these entries: doing so would drop the very bare-import calls the
    /// clippy count exists to catch, and would silently lower committed
    /// baselines. This pins the classification the counting loop keys on.
    #[test]
    fn a_bare_import_call_is_a_textual_entry_the_count_still_records() {
        let entries = parse_deny_list(
            "disallowed-methods = [\n    { path = \"std::fs::read_to_string\", reason = \"x\", replacement = \"tokio\" },\n    { path = \"tokio::runtime::Runtime::block_on\", reason = \"x\", replacement = \"async\" },\n]\n",
        );
        let by_path: BTreeMap<&str, EntryKind> = entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.kind))
            .collect();
        assert_eq!(
            by_path["std::fs::read_to_string"],
            EntryKind::Textual,
            "a lowercase module segment makes the entry textual, so the count loop \
             does not require attribution for it"
        );
        assert_eq!(
            by_path["tokio::runtime::Runtime::block_on"],
            EntryKind::InstanceMethod,
            "an uppercase type segment makes the entry an instance method, which is \
             the class the phantom macro-generated calls inflated"
        );
    }

    /// The attributable case must still be found when the call is not in the
    /// FIRST span: attribution picks the span that renders the call.
    #[test]
    fn attribution_prefers_the_span_that_renders_the_call() {
        let entries = parse_deny_list(
            "disallowed-methods = [\n    { path = \"std::sync::Mutex::lock\", reason = \"x\", replacement = \"tokio\" },\n]\n",
        );
        let json = r#"
{"reason":"compiler-message","message":{"code":{"code":"clippy::disallowed_methods"},"message":"use of a disallowed method `std::sync::Mutex::lock`","spans":[{"file_name":"packages/d2b-broker/src/other.rs","line_start":3,"is_primary":true,"text":[{"text":"    unrelated()"}]},{"file_name":"packages/d2b-broker/src/runtime.rs","line_start":41,"is_primary":false,"text":[{"text":"    state.lock()"}]}]}}
"#;
        let hits = parse_clippy_disallowed(json, &entries);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].attributed, "the call is rendered by the second span");
        assert_eq!(hits[0].file, "packages/d2b-broker/src/runtime.rs");
        assert_eq!(hits[0].line, 41);
    }

    #[test]
    fn suppression_scan_splits_blanket_from_per_site_and_skips_comments() {
        let raw = "\
//! A doc comment mentioning #[allow(clippy::disallowed_methods)] must not count.
#![allow(clippy::disallowed_methods)]

pub fn worker() {
    // A comment: #[expect(clippy::disallowed_methods)] still not a site.
    let s = \"#[allow(clippy::disallowed_methods)] in a string, not a site\";
}

#[allow(clippy::disallowed_methods, reason = \"dedicated bounded worker per plan R4\")]
pub fn recv_loop() {}

#[expect(
    clippy::disallowed_methods,
    reason = \"cfg(test) helper\"
)]
pub fn helper() {}

#[allow(dead_code, clippy::await_holding_lock)]
pub fn other() {}
";
        let sites = scan_suppressions("fixture.rs", raw);
        assert_eq!(sites.len(), 4, "sites: {sites:?}");
        let blanket = sites.iter().filter(|site| site.blanket).collect::<Vec<_>>();
        assert_eq!(blanket.len(), 1);
        assert_eq!(blanket[0].lint, "clippy::disallowed_methods");
        assert_eq!(blanket[0].line, 2);
        assert!(blanket[0].reason.is_none());

        let per_site = sites.iter().filter(|site| !site.blanket).collect::<Vec<_>>();
        assert_eq!(per_site.len(), 3);
        let worker = per_site
            .iter()
            .find(|site| site.reason.as_deref() == Some("dedicated bounded worker per plan R4"))
            .expect("worker allow with reason");
        assert!(!worker.expect);
        let helper = per_site
            .iter()
            .find(|site| site.expect && site.reason.as_deref() == Some("cfg(test) helper"))
            .expect("expect with reason");
        assert!(!helper.blanket);
        let hold = per_site
            .iter()
            .find(|site| site.lint == "clippy::await_holding_lock")
            .expect("multi-lint allow");
        assert_eq!(hold.line, 18);
    }

    #[test]
    fn workspace_member_paths_parse_the_committed_manifest() {
        // Runtime lookup (not env!): Bazel's process_wrapper forbids
        // embedding CARGO_MANIFEST_DIR, and under Bazel the variable is
        // unset, so this package test resolves the repo root from the
        // current working directory instead (cargo runs integration tests
        // from the workspace root).
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok();
        let root = manifest_dir
            .map(|dir| Path::new(&dir).join("../.."))
            .unwrap_or_else(|| std::env::current_dir().expect("current dir"));
        let members = workspace_member_paths(&root).expect("parse root manifest");
        assert!(members.contains("packages/d2b-broker"));
        assert!(members.contains("packages/xtask"));
    }

    #[test]
    fn suppression_scan_ignores_lifetimes_and_raw_strings() {
        let raw = "\
pub fn f<'a>(x: &'a str) -> &'a str {
    let raw = r#\"#[allow(clippy::disallowed_methods)]\"#;
    let c = 'x';
    let _ = (raw, c, x);
    x
}
";
        let sites = scan_suppressions("fixture.rs", raw);
        assert!(sites.is_empty(), "no real attributes: {sites:?}");
    }

    #[test]
    fn clippy_command_matches_the_documented_contract() {
        // The de-escalation set after `--` is the contract documented in the
        // root Cargo.toml ("Keep the census visible:"); a drift here would
        // let a deny-level `await_holding_lock`/`await_holding_refcell_ref`
        // diagnostic fail the gate off the clippy-run error path.
        let command = clippy_command(Path::new("/repo"), &[]);
        let args: Vec<&str> = command.get_args().map(|arg| arg.to_str().unwrap()).collect();
        assert_eq!(
            args,
            [
                "clippy",
                "--locked",
                "--message-format=json",
                "--all-targets",
                "--workspace",
                "--",
                "-W",
                "warnings",
                "-W",
                "clippy::disallowed_methods",
                "-W",
                "clippy::await_holding_lock",
                "-W",
                "clippy::await_holding_refcell_ref",
            ]
        );
        let per_crate = clippy_command(Path::new("/repo"), &["d2b-broker".to_string()]);
        let args: Vec<&str> = per_crate.get_args().map(|arg| arg.to_str().unwrap()).collect();
        assert_eq!(
            args,
            [
                "clippy",
                "--locked",
                "--message-format=json",
                "--all-targets",
                "-p",
                "d2b-broker",
                "--",
                "-W",
                "warnings",
                "-W",
                "clippy::disallowed_methods",
                "-W",
                "clippy::await_holding_lock",
                "-W",
                "clippy::await_holding_refcell_ref",
            ]
        );
    }

    #[test]
    fn first_diagnostic_surfaces_the_first_real_error_not_the_summary_tail() {
        let stderr = "\
error[E0308]: mismatched types
   --> packages/d2b-broker/tests/planted.rs:2:17
    |
2   |     let x: u32 = \"boom\";
    |              ^^ expected `u32`, found `&str`
    |
error: could not compile `d2b-broker` (test \"planted\") due to 1 previous error
warning: build failed, waiting for other jobs to finish...
";
        assert_eq!(
            first_diagnostic(stderr),
            Some(
                "packages/d2b-broker/tests/planted.rs:2: mismatched types".to_string()
            )
        );
    }

    #[test]
    fn first_diagnostic_prefers_errors_over_leading_warnings() {
        let stderr = "\
warning: unused variable: `x`
 --> packages/d2b-broker/src/lib.rs:10:9
  |
error[E0425]: cannot find value `nope` in this scope
 --> packages/d2b-broker/src/runtime.rs:41:13
  |
error: aborting due to previous error
";
        assert_eq!(
            first_diagnostic(stderr),
            Some(
                "packages/d2b-broker/src/runtime.rs:41: cannot find value `nope` in this scope"
                    .to_string()
            )
        );
    }

    #[test]
    fn first_diagnostic_reads_through_message_continuation_lines() {
        let stderr = "\
error[E0277]: the trait bound `Foo: Bar` is not satisfied
             the following other types implement trait `Bar`:
               `Baz`
   --> packages/d2b-broker/src/lib.rs:3:5
  |
error: could not compile `d2b-broker` due to previous error
";
        assert_eq!(
            first_diagnostic(stderr),
            Some(
                "packages/d2b-broker/src/lib.rs:3: the trait bound `Foo: Bar` is not satisfied"
                    .to_string()
            )
        );
    }

    #[test]
    fn first_diagnostic_falls_back_to_spanless_headers_and_none() {
        let cargo_level = "error: failed to parse manifest at `/repo/Cargo.toml`\n\nCaused by:\n  no `package` section found.\n";
        assert_eq!(
            first_diagnostic(cargo_level),
            Some("failed to parse manifest at `/repo/Cargo.toml`".to_string())
        );
        assert_eq!(first_diagnostic(""), None);
        assert_eq!(first_diagnostic("  Compiling d2b-broker v0.0.0-bootstrap\n"), None);
    }

    #[test]
    fn first_json_diagnostic_surfaces_the_first_error_with_its_primary_span() {
        // The shape cargo clippy --message-format=json actually emits: the
        // diagnostic lives only in the JSON stream, stderr carries just the
        // cargo summaries.
        let json = r#"
{"reason":"compiler-message","message":{"level":"error","message":"mismatched types","spans":[{"file_name":"packages/d2b-broker/tests/planted_compile_error_585.rs","line_start":4,"is_primary":true},{"file_name":"packages/d2b-broker/tests/planted_compile_error_585.rs","line_start":4,"is_primary":false}]}}
{"reason":"compiler-message","message":{"level":"warning","message":"unused variable: `x`","spans":[{"file_name":"packages/d2b-broker/src/lib.rs","line_start":10,"is_primary":true}]}}
{"reason":"build-finished","success":false}
"#;
        assert_eq!(
            first_json_diagnostic(json),
            Some(
                "packages/d2b-broker/tests/planted_compile_error_585.rs:4: mismatched types"
                    .to_string()
            )
        );
    }

    #[test]
    fn first_json_diagnostic_prefers_errors_and_ignores_spanless_messages() {
        let json = r#"
{"reason":"compiler-message","message":{"level":"warning","message":"unused variable: `x`","spans":[{"file_name":"packages/d2b-broker/src/lib.rs","line_start":10,"is_primary":true}]}}
{"reason":"compiler-message","message":{"level":"error","message":"cannot find value `nope` in this scope","spans":[{"file_name":"packages/d2b-broker/src/runtime.rs","line_start":41,"is_primary":true}]}}
{"reason":"compiler-message","message":{"level":"error","message":"aborting due to previous error","spans":[]}}
{"reason":"build-finished","success":false}
"#;
        assert_eq!(
            first_json_diagnostic(json),
            Some(
                "packages/d2b-broker/src/runtime.rs:41: cannot find value `nope` in this scope"
                    .to_string()
            )
        );
        let warnings_only = r#"
{"reason":"compiler-message","message":{"level":"warning","message":"unused variable: `x`","spans":[{"file_name":"packages/d2b-broker/src/lib.rs","line_start":10,"is_primary":true}]}}
"#;
        assert_eq!(
            first_json_diagnostic(warnings_only),
            Some("packages/d2b-broker/src/lib.rs:10: unused variable: `x`".to_string())
        );
        assert_eq!(first_json_diagnostic(""), None);
        assert_eq!(
            first_json_diagnostic("{\"reason\":\"build-finished\",\"success\":false}\n"),
            None,
            "a cargo-level failure without compiler messages has no diagnostic"
        );
    }
    #[test]
    fn run_clippy_consults_the_stderr_cargo_error_before_a_warning_only_json_pick() {
        // The JSON stream holds only a warning-with-span; the true cause is
        // a cargo-level failure (`error: failed to run custom build
        // command`) that never reached rustc and lives in the stderr text.
        let json = r#"
{"reason":"compiler-message","message":{"level":"warning","message":"unused variable: `x`","spans":[{"file_name":"packages/d2b-broker/src/lib.rs","line_start":10,"is_primary":true}]}}
"#;
        let stderr = "error: failed to run custom build command for `d2b-broker`
";
        assert_eq!(
            first_json_error(json),
            None,
            "a warning-only JSON stream has no error-level pick"
        );
        assert_eq!(
            first_stderr_error(stderr),
            Some("failed to run custom build command for `d2b-broker`".to_string())
        );
        assert_eq!(
            first_json_error(json).or_else(|| first_stderr_error(stderr)),
            Some("failed to run custom build command for `d2b-broker`".to_string()),
            "the composed error-first chain picks the stderr cargo error over the warning-only JSON pick"
        );
    }

    #[test]
    fn first_json_error_prefers_the_spanless_error_over_a_warning_with_span() {
        // An error-level record without a primary span outranks any
        // warning-level record that carries one - the run only fails on
        // errors, so a cargo-level failure reported spanless is the cause.
        let json = r#"
{"reason":"compiler-message","message":{"level":"error","message":"failed to run custom build command for `d2b-broker`","spans":[]}}
{"reason":"compiler-message","message":{"level":"warning","message":"unused variable: `x`","spans":[{"file_name":"packages/d2b-broker/src/lib.rs","line_start":10,"is_primary":true}]}}
"#;
        assert_eq!(
            first_json_error(json),
            Some("failed to run custom build command for `d2b-broker`".to_string())
        );
        assert_eq!(
            first_json_diagnostic(json),
            Some("failed to run custom build command for `d2b-broker`".to_string())
        );
    }

}