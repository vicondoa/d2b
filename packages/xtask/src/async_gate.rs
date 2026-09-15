//! Async-discipline gate (U13, KTD9).
//!
//! Rejects blocking calls on Tokio runtime workers. The known blocking
//! functions are the `clippy.toml` `disallowed-methods` deny list - the same
//! single source of truth the blocking census uses, so the gate and the lint
//! cannot drift - and the probe is a lexical scan: a call to a denied API
//! inside an async context (`async fn` body or `async`/`async move` block) is
//! a violation, because the call runs on an executor worker and stalls every
//! task scheduled on it.
//!
//! The scan is deliberately lexical, not a parse, and fails closed:
//!
//! * A `{` whose enclosing statement names `async fn` opens an async context;
//!   an `async`/`async move` block opens one too. A nested block inside an
//!   async context stays async - it still runs on the worker.
//! * The one sanctioned escape is the KTD9 adapter: a `spawn_blocking(...)`
//!   argument runs on a blocking thread, so its body is exempt. Every other
//!   blocking call inside an async context is reported, including one inside
//!   a `std::thread::spawn` closure (a raw thread per call is itself the
//!   shape the deny list's replacement vocabulary rejects).
//! * `#[allow(clippy::disallowed_methods)]` does not exempt a call: the
//!   deny list reserves that allow for genuinely synchronous paths, and no
//!   synchronous path belongs inside an async context.
//! * Test code is scanned like production code: `#[tokio::test]` bodies run
//!   on a runtime, so a blocking call there starves workers too.
//!
//! Known limitations of the lexical form, all fail-closed or documented:
//!
//! * A call through an imported item (`use std::fs::read;` then `read(...)`)
//!   has no textual path to match; the clippy lint catches it once the
//!   deny list flips to `deny`. An import *inside* an async context is
//!   reported instead, which is the fail-closed direction.
//! * `use tokio::fs;` followed by a bare `fs::read(...)` is textually
//!   indistinguishable from the `std` import form and is reported; the
//!   house replacement vocabulary is the fully qualified `tokio::fs::read`,
//!   which never matches.
//! * A method-call form (`mutex.lock()`) does not match the qualified path
//!   the deny list names; the qualified form and the `X::lock(...)` form do.
//!
//! The gate is a runnable subcommand (`cargo xtask check-async-gate
//! [<paths>...]`) with fixture unit tests; wiring it into the enforcement
//! chain is the final step of U13 on the migrated tree.

use std::fs;
use std::path::{Path, PathBuf};

use crate::blocking_census::{parse_deny_list, DeniedApi};

/// The named violation class every finding renders under.
pub const VIOLATION_NAME: &str = "blocking-call-in-async-context";

/// The default scan roots: the broker, the daemon, and every provider crate
/// (the handler crates linked into the broker binary - a non-yielding handler
/// starves the whole envelope).
const DEFAULT_CRATE_ROOTS: &[&str] = &["packages/d2b-broker", "packages/d2bd"];

/// One blocking call found on a runtime worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// Repository-relative path of the file, as given to [`scan_source`].
    pub file: String,
    /// 1-based line number of the call.
    pub line: usize,
    /// The denied API from `clippy.toml` (for example `std::fs::read`).
    pub api: String,
    /// `"async fn"` or `"async block"` - the async context the call sits in.
    pub context: &'static str,
    /// The offending line, trimmed.
    pub code: String,
}

impl Violation {
    /// Render the violation as canonical JSON for the gate report.
    pub fn render(&self) -> String {
        serde_json::json!({
            "violation": VIOLATION_NAME,
            "file": self.file,
            "line": self.line,
            "api": self.api,
            "context": self.context,
            "code": self.code,
        })
        .to_string()
    }
}

/// What one open brace level is: the worker context its body runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BraceKind {
    /// A synchronous level; calls here are not on a runtime worker.
    Sync,
    /// The body of an `async fn`.
    AsyncFn,
    /// The body of an `async { ... }` / `async move { ... }` block.
    AsyncBlock,
    /// A level inside a `spawn_blocking(...)` argument; runs on a blocking
    /// thread, never on a worker.
    Blocking,
}

/// Lexical scanner state across a file's lines.
struct ScanState {
    /// One entry per open brace level, innermost last.
    braces: Vec<BraceKind>,
    /// One entry per open parenthesis: whether it is a `spawn_blocking` call.
    parens: Vec<bool>,
    /// Whether the scanner is inside a `/* ... */` comment spanning lines.
    in_block_comment: bool,
    /// Hash count of an unterminated `r#"..."#` string spanning lines.
    raw_hashes: Option<usize>,
    /// The code since the last `;`, `{`, or `}` - used to recognize `async
    /// fn` signatures and the token before a `{` or `(`.
    segment: String,
}

impl ScanState {
    fn new() -> Self {
        Self {
            braces: Vec::new(),
            parens: Vec::new(),
            in_block_comment: false,
            raw_hashes: None,
            segment: String::new(),
        }
    }
}

/// Scan one source file for blocking calls inside async contexts.
pub fn scan_source(file: &str, text: &str, entries: &[DeniedApi]) -> Vec<Violation> {
    let mut state = ScanState::new();
    let mut violations = Vec::new();
    for (index, line) in text.lines().enumerate() {
        scan_line(&mut state, file, index + 1, line, entries, &mut violations);
    }
    violations
}

/// Scan one line, advancing the scanner state and collecting violations.
fn scan_line(
    state: &mut ScanState,
    file: &str,
    line_no: usize,
    raw: &str,
    entries: &[DeniedApi],
    out: &mut Vec<Violation>,
) {
    let code = sanitize(raw, state);
    let bytes = code.as_bytes();
    let mut i = 0;
    let mut flagged: Vec<&str> = Vec::new();
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'{' => {
                state.braces.push(brace_kind(state));
                state.segment.clear();
            }
            b'}' => {
                state.braces.pop();
                state.segment.clear();
            }
            b'(' => {
                state
                    .parens
                    .push(last_token(&state.segment) == Some("spawn_blocking"));
            }
            b')' => {
                state.parens.pop();
            }
            b';' => {
                state.segment.clear();
            }
            _ => {
                let context = match state.braces.last() {
                    Some(BraceKind::AsyncFn) => Some("async fn"),
                    Some(BraceKind::AsyncBlock) => Some("async block"),
                    _ => None,
                };
                let on_worker = !state.parens.iter().any(|blocking| *blocking);
                let token_start = i == 0 || !is_ident_char(bytes[i - 1]);
                if let Some(context) = context
                    && on_worker
                    && token_start
                {
                    for entry in entries {
                        if flagged.contains(&entry.path.as_str()) {
                            continue;
                        }
                        let first = entry.path.as_bytes()[0];
                        if first != b && entry.tail.as_bytes()[0] != b {
                            continue;
                        }
                        if matches_at(&code, i, entry) {
                            flagged.push(&entry.path);
                            out.push(Violation {
                                file: file.to_owned(),
                                line: line_no,
                                api: entry.path.clone(),
                                context,
                                code: raw.trim().to_owned(),
                            });
                        }
                    }
                }
                let ch = code[i..].chars().next().expect("i stays on a char boundary");
                state.segment.push(ch);
                i += ch.len_utf8();
                continue;
            }
        }
        i += 1;
    }
}

/// Whether a denied API matches at byte position `i` of the sanitized code.
///
/// The full path matches when it is not glued to a surrounding identifier.
/// The tail (the last two path segments, the distinctive text form clippy
/// matches on) additionally requires that it is not glued to a preceding
/// `::` - that is what keeps `tokio::fs::read` and the other sanctioned
/// replacements from matching `std::fs::read`'s tail.
fn matches_at(code: &str, i: usize, entry: &DeniedApi) -> bool {
    let bytes = code.as_bytes();
    let path = &entry.path;
    if code[i..].starts_with(path.as_str()) {
        let after = i + path.len();
        let before_ok = i == 0 || !is_ident_char(bytes[i - 1]);
        let after_ok = after == bytes.len() || !is_ident_char(bytes[after]);
        if before_ok && after_ok {
            return true;
        }
    }
    let tail = &entry.tail;
    if code[i..].starts_with(tail.as_str()) {
        let after = i + tail.len();
        let before_ok =
            i == 0 || (bytes[i - 1] != b':' && !is_ident_char(bytes[i - 1]));
        let after_ok = after == bytes.len() || !is_ident_char(bytes[after]);
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// The kind of a `{` opening at the current scanner position.
fn brace_kind(state: &ScanState) -> BraceKind {
    if state.parens.iter().any(|blocking| *blocking) {
        return BraceKind::Blocking;
    }
    if segment_has_async_fn(&state.segment) {
        return BraceKind::AsyncFn;
    }
    match last_token(&state.segment) {
        Some("async") => return BraceKind::AsyncBlock,
        Some("move") if second_last_token(&state.segment) == Some("async") => {
            return BraceKind::AsyncBlock;
        }
        _ => {}
    }
    match state.braces.last() {
        Some(BraceKind::AsyncFn) => BraceKind::AsyncFn,
        Some(BraceKind::AsyncBlock) => BraceKind::AsyncBlock,
        _ => BraceKind::Sync,
    }
}

/// Whether the current segment opens an `async fn` body.
///
/// The signature can span lines, so the segment accumulates until the body
/// brace; `async fn_pointer()` must not match (the `fn` needs a word
/// boundary after it).
fn segment_has_async_fn(segment: &str) -> bool {
    segment.match_indices("async fn").any(|(pos, _)| {
        let before_ok = pos == 0 || !is_ident_char(segment.as_bytes()[pos - 1]);
        let after = pos + "async fn".len();
        let after_ok = after == segment.len() || !is_ident_char(segment.as_bytes()[after]);
        before_ok && after_ok
    })
}

/// The identifier token ending at `offset` (exclusive), skipping whitespace.
fn token_before(segment: &str, offset: usize) -> Option<&str> {
    let bytes = segment.as_bytes();
    let mut end = offset;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && is_ident_char(bytes[start - 1]) {
        start -= 1;
    }
    if start == end {
        None
    } else {
        Some(&segment[start..end])
    }
}

/// The last identifier token of the segment.
fn last_token(segment: &str) -> Option<&str> {
    token_before(segment, segment.len())
}

/// The identifier token before the last one.
fn second_last_token(segment: &str) -> Option<&str> {
    let last = last_token(segment)?;
    token_before(segment, segment.len() - last.len())
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// How a line is being consumed while sanitizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    String,
    Char,
    BlockComment,
}

/// The code half of one line: comments and string/char literal contents
/// replaced with spaces, so neither a comment mention nor a literal can
/// match an API path and braces inside them cannot move the scanner.
///
/// Multi-line `/* ... */` comments and `r#"..."#` strings carry their state
/// across lines.
fn sanitize(line: &str, state: &mut ScanState) -> String {
    let bytes = line.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut mode = if state.in_block_comment {
        Mode::BlockComment
    } else {
        Mode::Normal
    };
    state.in_block_comment = false;
    let mut raw_hashes = state.raw_hashes.take();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match mode {
            Mode::BlockComment => {
                if b == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    out.extend_from_slice(b"  ");
                    i += 2;
                    mode = Mode::Normal;
                } else {
                    out.push(b' ');
                    i += 1;
                }
            }
            Mode::String => {
                if b == b'\\' {
                    out.extend_from_slice(b"  ");
                    i += 2;
                } else if b == b'"' {
                    out.push(b' ');
                    i += 1;
                    mode = Mode::Normal;
                } else {
                    out.push(b' ');
                    i += 1;
                }
            }
            Mode::Char => {
                if b == b'\\' {
                    out.extend_from_slice(b"  ");
                    i += 2;
                } else if b == b'\'' {
                    out.push(b' ');
                    i += 1;
                    mode = Mode::Normal;
                } else {
                    out.push(b' ');
                    i += 1;
                }
            }
            Mode::Normal => {
                if let Some(hashes) = raw_hashes {
                    if b == b'"' && raw_string_closes(&bytes[i..], hashes) {
                        out.extend(std::iter::repeat_n(b' ', hashes + 1));
                        i += hashes + 1;
                        raw_hashes = None;
                    } else {
                        out.push(b' ');
                        i += 1;
                    }
                } else if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
                    out.extend(vec![b' '; bytes.len() - i]);
                    break;
                } else if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    out.extend_from_slice(b"  ");
                    i += 2;
                    mode = Mode::BlockComment;
                } else if b == b'"' {
                    if let Some(hashes) = raw_string_hashes(&bytes[..i]) {
                        raw_hashes = Some(hashes);
                        out.push(b' ');
                        i += 1;
                    } else {
                        out.push(b' ');
                        i += 1;
                        mode = Mode::String;
                    }
                } else if b == b'\'' {
                    if char_literal_starts(&bytes[i..]) {
                        out.push(b' ');
                        i += 1;
                        mode = Mode::Char;
                    } else {
                        out.push(b);
                        i += 1;
                    }
                } else {
                    out.push(b);
                    i += 1;
                }
            }
        }
    }
    if mode == Mode::BlockComment {
        state.in_block_comment = true;
    }
    if raw_hashes.is_some() {
        state.raw_hashes = raw_hashes;
    }
    String::from_utf8(out).expect("sanitized line stays valid utf-8")
}

/// The hash count of a raw string whose opening `"` is at the end of
/// `before_quote` (`r#"`, `br#"`, `cr#"`), when the prefix is a real raw
/// string opener rather than an identifier.
fn raw_string_hashes(before_quote: &[u8]) -> Option<usize> {
    let mut hashes = 0;
    let mut j = before_quote.len();
    while j > 0 && before_quote[j - 1] == b'#' {
        hashes += 1;
        j -= 1;
    }
    if hashes == 0 {
        return None;
    }
    let mut k = j;
    let mut prefix_len = 0;
    while k > 0 && prefix_len < 2 && is_ident_char(before_quote[k - 1]) {
        k -= 1;
        prefix_len += 1;
    }
    let prefix = &before_quote[k..j];
    if prefix != b"r" && prefix != b"b" && prefix != b"c" && prefix != b"br" && prefix != b"cr" {
        return None;
    }
    if k > 0 && is_ident_char(before_quote[k - 1]) {
        return None;
    }
    Some(hashes)
}

/// Whether the bytes at the start of `bytes` close a raw string opened with
/// `hashes` hashes: a `"` followed by exactly `hashes` `#`s.
fn raw_string_closes(bytes: &[u8], hashes: usize) -> bool {
    let after = 1 + hashes;
    bytes.len() >= after && bytes[1..after].iter().all(|b| *b == b'#')
}

/// Whether a `'` at the start of `bytes` opens a char literal rather than a
/// lifetime. A lifetime is an identifier run with no closing quote; an
/// escaped or non-identifier character is a char literal.
fn char_literal_starts(bytes: &[u8]) -> bool {
    let Some(&next) = bytes.get(1) else {
        return false;
    };
    if next == b'\\' {
        return true;
    }
    if is_ident_char(next) {
        let mut end = 1;
        while end < bytes.len() && is_ident_char(bytes[end]) {
            end += 1;
        }
        return bytes.get(end) == Some(&b'\'');
    }
    true
}

/// Load the deny list - the single source of truth for blocking functions.
fn load_entries(repo_root: &Path) -> Result<Vec<DeniedApi>, String> {
    let clippy_toml = fs::read_to_string(repo_root.join("clippy.toml"))
        .map_err(|_| "async-gate: clippy.toml unreadable".to_owned())?;
    let entries = parse_deny_list(&clippy_toml);
    if entries.is_empty() {
        return Err("async-gate: no entries parsed from clippy.toml".to_owned());
    }
    Ok(entries)
}

/// The default scan roots: the broker, the daemon, and every provider crate.
fn default_scan_paths(repo_root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for root in DEFAULT_CRATE_ROOTS {
        let path = repo_root.join(root);
        if path.is_dir() {
            paths.push(path);
        }
    }
    let packages = repo_root.join("packages");
    let entries = fs::read_dir(&packages)
        .map_err(|error| format!("async-gate: read dir {}: {error}", packages.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("async-gate: dir entry: {error}"))?;
        if entry.path().is_dir()
            && entry.file_name().to_string_lossy().starts_with("d2b-provider-")
        {
            paths.push(entry.path());
        }
    }
    Ok(paths)
}

/// Scan every `.rs` file under the given paths (files or directories).
fn scan_paths(
    repo_root: &Path,
    paths: &[PathBuf],
    entries: &[DeniedApi],
) -> Result<(Vec<Violation>, usize), String> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            walk_rs(path, &mut files)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path.clone());
        }
    }
    files.sort();
    let mut violations = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file)
            .map_err(|error| format!("async-gate: read {}: {error}", file.display()))?;
        let display = file
            .strip_prefix(repo_root)
            .map(|relative| relative.display().to_string())
            .unwrap_or_else(|_| file.display().to_string());
        violations.extend(scan_source(&display, &text, entries));
    }
    Ok((violations, files.len()))
}

fn walk_rs(dir: &Path, found: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|error| format!("async-gate: read dir {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("async-gate: dir entry: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            walk_rs(&path, found)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
    Ok(())
}

/// Run the gate: scan the default roots (or the given paths) and fail on any
/// blocking call inside an async context.
pub fn run(repo_root: &Path, args: &[String]) -> Result<(), String> {
    let entries = load_entries(repo_root)?;
    let paths = if args.is_empty() {
        default_scan_paths(repo_root)?
    } else {
        args.iter().map(PathBuf::from).collect()
    };
    let (violations, file_count) = scan_paths(repo_root, &paths, &entries)?;
    if violations.is_empty() {
        println!(
            "async gate: {file_count} file(s) scanned, no blocking calls in async contexts"
        );
        Ok(())
    } else {
        let rendered = violations
            .iter()
            .map(Violation::render)
            .collect::<Vec<_>>()
            .join("\n");
        Err(format!(
            "{} blocking call(s) in async context(s) across {file_count} file(s):\n{rendered}",
            violations.len()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deny_list() -> Vec<DeniedApi> {
        parse_deny_list(include_str!("../../../clippy.toml"))
    }

    const ERROR_FIXTURE: &str = "packages/d2b-broker/src/worker.rs";

    #[test]
    fn flags_blocking_calls_in_async_contexts() {
        let violations = scan_source(
            ERROR_FIXTURE,
            include_str!("../tests/fixtures/async-gate/blocking_in_async_fn.rs"),
            &deny_list(),
        );
        let mut found: Vec<(&str, &str)> = violations
            .iter()
            .map(|violation| (violation.api.as_str(), violation.context))
            .collect();
        found.sort_unstable();
        assert_eq!(
            found,
            vec![
                ("std::fs::read", "async fn"),
                ("std::fs::write", "async block"),
                ("std::sync::Mutex::lock", "async fn"),
                ("std::thread::sleep", "async fn"),
            ],
            "the fixture's four blocking calls must each be flagged exactly once"
        );
        for violation in &violations {
            assert_eq!(violation.file, ERROR_FIXTURE);
            assert!(!violation.code.is_empty());
        }
    }

    #[test]
    fn violations_render_as_named_json() {
        let violations = scan_source(
            ERROR_FIXTURE,
            include_str!("../tests/fixtures/async-gate/blocking_in_async_fn.rs"),
            &deny_list(),
        );
        let rendered = violations[0].render();
        assert!(rendered.contains("\"violation\":\"blocking-call-in-async-context\""));
        assert!(rendered.contains("\"file\":\"packages/d2b-broker/src/worker.rs\""));
        assert!(rendered.contains("\"api\":\"std::fs::read\""));
        assert!(rendered.contains("\"context\":\"async fn\""));
    }

    #[test]
    fn passes_async_contexts_without_blocking_calls() {
        let violations = scan_source(
            "packages/d2b-broker/src/clean.rs",
            include_str!("../tests/fixtures/async-gate/clean_async.rs"),
            &deny_list(),
        );
        assert!(
            violations.is_empty(),
            "clean fixture must pass: {:?}",
            violations.iter().map(Violation::render).collect::<Vec<_>>()
        );
    }

    #[test]
    fn sync_functions_are_not_worker_contexts() {
        let source = "pub fn load(path: &Path) -> Vec<u8> {\n    std::fs::read(path).unwrap_or_default()\n}\n";
        let violations = scan_source("x.rs", source, &deny_list());
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn spawn_blocking_bodies_are_exempt_even_on_one_line() {
        let source = "pub async fn load(path: &Path) -> Vec<u8> {\n    tokio::task::spawn_blocking(|| std::fs::read(path)).await.unwrap_or_default()\n}\n";
        let violations = scan_source("x.rs", source, &deny_list());
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn comment_mentions_do_not_flag() {
        let source = "pub async fn load() {\n    // std::fs::read blocks; tokio::fs::read does not\n    tokio::fs::read(\"/x\").await.unwrap();\n}\n";
        let violations = scan_source("x.rs", source, &deny_list());
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn tokio_replacements_do_not_flag() {
        let source = "pub async fn load(m: &tokio::sync::Mutex<u32>) {\n    tokio::fs::read(\"/x\").await.unwrap();\n    tokio::time::sleep(std::time::Duration::from_secs(1)).await;\n    tokio::sync::Mutex::lock(m).await;\n}\n";
        let violations = scan_source("x.rs", source, &deny_list());
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn imported_module_calls_flag_via_the_tail() {
        let source = "use std::fs;\n\npub async fn load() {\n    fs::read(\"/x\").unwrap();\n}\n";
        let violations = scan_source("x.rs", source, &deny_list());
        assert!(
            violations.iter().any(|violation| violation.api == "std::fs::read"),
            "imported-module call must flag: {violations:?}"
        );
    }

    #[test]
    fn multi_line_async_fn_signatures_stay_async() {
        let source = "pub async fn load(\n    path: &Path,\n) -> Vec<u8> {\n    std::fs::read(path).unwrap_or_default()\n}\n";
        let violations = scan_source("x.rs", source, &deny_list());
        assert!(
            violations.iter().any(|violation| violation.api == "std::fs::read"),
            "a signature split across lines must still be an async context: {violations:?}"
        );
    }

    #[test]
    fn nested_blocks_inherit_the_async_context() {
        let source = "pub async fn load(path: &Path) {\n    if path.exists() {\n        std::fs::read(path).unwrap();\n    }\n}\n";
        let violations = scan_source("x.rs", source, &deny_list());
        assert!(
            violations.iter().any(|violation| violation.api == "std::fs::read"),
            "a block nested in an async fn still runs on the worker: {violations:?}"
        );
    }

    #[test]
    fn async_fn_pointer_does_not_open_an_async_context() {
        let source = "pub fn make() {\n    let g = async fn_pointer();\n    if g {\n        std::fs::read(\"/x\").unwrap();\n    }\n}\n";
        let violations = scan_source("x.rs", source, &deny_list());
        assert!(violations.is_empty(), "{violations:?}");
    }
}