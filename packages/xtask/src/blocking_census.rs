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
//! `disallowed_methods` diagnostics (run with `-W clippy::disallowed_methods`
//! so the manifest's `allow` level does not hide the diagnostics while
//! `#[allow]` attributes keep suppressing - the same predicate the lint
//! enforces under deny). The lexical meter is the report's cross-check:
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
    pub path: String,
    pub tail: String,
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
}

/// Parse `cargo clippy --message-format=json` output and return the
/// `disallowed_methods` diagnostics attributed to deny-list entries. A
/// diagnostic is attributed by the backtick-delimited configured path the
/// lint names in its message, so `Receiver::recv_timeout` never counts as
/// `Receiver::recv`.
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
        let span = message
            .get("spans")
            .and_then(|s| s.as_array())
            .and_then(|spans| spans.first());
        let file = span
            .and_then(|s| s.get("file_name"))
            .and_then(|f| f.as_str())
            .unwrap_or_default()
            .to_owned();
        let line_no = span
            .and_then(|s| s.get("line_start"))
            .and_then(|l| l.as_u64())
            .unwrap_or(0) as usize;
        hits.push(ClippyHit {
            entry_path: entry.path.clone(),
            file,
            line: line_no,
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
}

/// The committed per-crate baseline the CI cap compares against (plan R15).
///
/// The map is keyed by crate directory (`packages/d2b-broker`) then by
/// deny-list entry path, so every deny-entry class - including
/// `tokio::task::spawn_blocking` - carries its own per-crate cap: the
/// no-new-spawn_blocking guard during the conversion window is the
/// spawn_blocking row, which the gate refuses to see grow.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CensusBaseline {
    pub crates: BTreeMap<String, BTreeMap<String, usize>>,
}

/// One source file under a census crate.
struct CensusFile {
    /// Repository-relative path.
    rel: String,
    split: SplitContext,
}

/// The workspace member paths the root manifest declares, e.g.
/// `packages/d2b-broker`.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn workspace_member_paths(repo_root: &Path) -> Result<BTreeSet<String>, String> {
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
/// Non-member directories under `packages/` (e.g. `d2b-realm-core`) are not
/// censused: they are not workspace crates, so the workspace-wide clippy run
/// cannot measure their instance-method classes.
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

/// The package name a crate directory's manifest declares.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn package_name(crate_dir: &Path) -> Result<String, String> {
    let manifest = fs::read_to_string(crate_dir.join("Cargo.toml"))
        .map_err(|error| format!("blocking-census: read {}: {error}", crate_dir.display()))?;
    manifest
        .lines()
        .find_map(|line| line.trim().strip_prefix("name = \""))
        .and_then(|name| name.strip_suffix('"'))
        .map(str::to_owned)
        .ok_or_else(|| format!("blocking-census: no package name in {}", crate_dir.display()))
}

/// Run `cargo clippy --message-format=json` over the given packages (or the
/// whole workspace) with `disallowed_methods` forced to warn, and return the
/// JSON stream. `RUSTFLAGS` is cleared so the `.cargo/config.toml`
/// `-D warnings` rustflags cannot turn unrelated warnings into failures, and
/// `-W` (not `--force-warn`) is used so `#[allow]` attributes keep
/// suppressing - the same predicate the lint enforces under deny.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn run_clippy(repo_root: &Path, packages: &[String]) -> Result<String, String> {
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
    command.arg("--").arg("-W").arg("clippy::disallowed_methods");
    let output = command
        .output()
        .map_err(|error| format!("blocking-census: cargo clippy launch failed: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(15).collect();
        return Err(format!(
            "blocking-census: cargo clippy failed (exit {}):\n{}",
            output.status,
            tail.join("\n")
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
/// split and suppression inventory.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn collect_crate_files(
    repo_root: &Path,
    crate_dir: &Path,
    files: &mut Vec<CensusFile>,
    blanket: &mut usize,
    per_site: &mut usize,
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

/// Run the census over the repository or the given crate paths. Prints the
/// per-crate tables and the totals; with `json_out` writes the authoritative
/// per-crate counts (the baseline shape); with `baseline` fails when any
/// covered crate's count exceeds its committed baseline (plan R15).
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
        collect_crate_files(repo_root, crate_dir, &mut files, &mut blanket, &mut per_site)?;

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
            let clippy_count = hits
                .iter()
                .filter(|hit| hit.entry_path == entry.path && hit.file.starts_with(&prefix))
                .count();
            let (production, test, source) = match entry.kind {
                EntryKind::Textual => (text_count.production, text_count.test, "lexical"),
                EntryKind::InstanceMethod => {
                    let in_production = hits
                        .iter()
                        .filter(|hit| {
                            hit.entry_path == entry.path
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

        crates.push(CrateCensus {
            crate_dir: rel_dir,
            package_name: package_name.clone(),
            counts,
            blanket_suppressions: blanket,
            per_site_suppressions: per_site,
        });
    }

    println!();
    println!("crates censused: {}", crates.len());
    println!("production blocking-API call sites: {production_total}");
    println!("test-context blocking-API call sites: {test_total}");
    println!("clippy-visible blocking-API call sites: {clippy_total}");
    println!("authoritative blocking-API call sites: {authoritative_total}");
    println!("deny-list entries: {}", entries.len());

    if let Some(path) = json_out {
        let baseline = CensusBaseline {
            crates: crates
                .iter()
                .map(|crate_census| (crate_census.crate_dir.clone(), crate_census.counts.clone()))
                .collect(),
        };
        let rendered = serde_json::to_string_pretty(&baseline)
            .map_err(|error| format!("blocking-census: serialize baseline: {error}"))?;
        fs::write(path, rendered + "\n")
            .map_err(|error| format!("blocking-census: write {}: {error}", path.display()))?;
        println!("baseline written: {}", path.display());
    }

    if let Some(path) = baseline {
        let committed = fs::read_to_string(path)
            .map_err(|error| format!("blocking-census: read baseline {}: {error}", path.display()))?;
        let committed: CensusBaseline = serde_json::from_str(&committed)
            .map_err(|error| format!("blocking-census: parse baseline {}: {error}", path.display()))?;
        let mut violations = Vec::new();
        for crate_census in &crates {
            let committed_counts = committed.crates.get(&crate_census.crate_dir);
            let mut crate_violations = Vec::new();
            for (entry, count) in &crate_census.counts {
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
            if crate_violations.is_empty() {
                println!(
                    "  {}: {} entry class(es) at or below baseline",
                    crate_census.crate_dir,
                    crate_census.counts.len()
                );
            } else {
                println!("  {}: ABOVE BASELINE", crate_census.crate_dir);
                for violation in &crate_violations {
                    println!("    {violation}");
                }
                violations.extend(crate_violations);
            }
        }
        if violations.is_empty() {
            println!("blocking-census check: PASS (no crate above its committed baseline)");
        } else {
            return Err(format!(
                "blocking-census check: FAILED - {} deny-entry class(es) above the committed baseline {}",
                violations.len(),
                path.display()
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let json = r#"
{"reason":"compiler-message","message":{"code":{"code":"clippy::disallowed_methods"},"message":"use of a disallowed method `std::sync::Mutex::lock`","spans":[{"file_name":"packages/d2b-broker/src/runtime.rs","line_start":41}]}}
{"reason":"compiler-message","message":{"code":{"code":"clippy::disallowed_methods"},"message":"use of a disallowed method `std::sync::mpsc::Receiver::recv_timeout`","spans":[{"file_name":"packages/d2b-broker/src/runtime.rs","line_start":42}]}}
{"reason":"compiler-message","message":{"code":{"code":"clippy::disallowed_methods"},"message":"use of a disallowed method `std::io::Write::write_all`","spans":[{"file_name":"packages/d2b-broker/src/audit.rs","line_start":7}]}}
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
        assert!(
            !members.contains("packages/d2b-realm-core"),
            "the excluded non-member crate must not be censused"
        );
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
}