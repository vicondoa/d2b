//! Async-discipline gate (U13, KTD9; method-call arming U6, KTD4).
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
//! * A `spawn_blocking(...)` argument body runs on a blocking thread, but the
//!   CALL itself is the banned thread-per-call shape that reaches the runtime's
//!   shared blocking pool (plan KD2), so since U13 the argument body is no
//!   longer exempt: it is still the caller's code and it parks the worker on the
//!   spawn/await rendezvous. Every blocking call inside an async context is
//!   reported, including one inside a `std::thread::spawn` closure (a raw
//!   thread per call is itself the shape the deny list's replacement vocabulary
//!   rejects).
//! * `#[allow(clippy::disallowed_methods)]` does not exempt a call: the
//!   deny list reserves that allow for genuinely synchronous paths, and no
//!   synchronous path belongs inside an async context.
//! * Test code is scanned like production code: `#[tokio::test]` bodies run
//!   on a runtime, so a blocking call there starves workers too.
//!
//! Since U6 (issue #590) the scanner also flags the conservative method-call
//! lock shape: a `lock()`/`read()`/`write()` method call inside an async
//! context that is NOT followed by `.await`. The production lock shape is the
//! method-call form (`mutex.lock()`), invisible to the qualified paths the
//! deny list names, so the gate was blind to exactly the shape it was built
//! to catch. The shape is deliberately conservative - the scanner does not
//! resolve receiver types, so any `.lock()`/`.read()`/`.write()` method call
//! in an async context matches - and the `.await` exclusion is load-bearing:
//! an awaited `tokio::sync::Mutex::lock()` site is legitimate (the census
//! never sees it either, since it counts only `clippy::disallowed_methods`
//! diagnostics), so only the un-awaited form is flagged.
//!
//! The escape hatch for a synchronous lock site that must stay is a
//! source-level marker: a trailing comment on the call's own line,
//! `// async-gate-allow: <reason>`, at or after the call. The marker format
//! and every marked site are recorded in the named hatch inventory
//! (`packages/xtask/data/async-gate-inventory.json`), and the gate validates
//! both directions: a marked site that is not recorded fails, and a recorded
//! site whose file no longer carries the marker fails too - as does a
//! recorded site whose file no longer exists in the tree (a deleted marked
//! file is stale in every scan mode; a file that exists but is outside the
//! current scan set is tolerated, so subset scans stay valid) - so the hatch
//! cannot drift into an allowlist. The marker exempts
//! only the method-call shape - the qualified form (`std::sync::Mutex::lock`)
//! and the `X::lock(...)` form have no hatch and always fail.
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
//! * The method-call shape is receiver-blind: a `.read()`/`.write()` on a
//!   non-lock receiver (a domain method, an `AsyncReadExt` call) is flagged
//!   like a mutex lock acquisition, and the marker is the documented way out.
//!   A method call whose closing paren is followed by `.await` on a later
//!   line is still exempt (the await decision spans lines).
//!
//! The gate is a runnable subcommand (`cargo xtask check-async-gate
//! [--write-inventory] [<paths>...]`) with fixture unit tests; it is wired
//! into the enforcement chain as the Layer-1 policy check `make
//! check-async-gate`. The inventory keys sites by `(file, line)`, so a
//! line-shifting edit above a marked call turns the gate red; the
//! `--write-inventory` mode regenerates the inventory from the run's marker
//! sites (over the default roots only) to repair exactly that drift.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::blocking_census::{parse_deny_list, DeniedApi};

/// The named violation class every finding renders under.
pub const VIOLATION_NAME: &str = "blocking-call-in-async-context";

/// The named hatch inventory (KTD4): records the source-level marker format
/// and every site the marker exempts, so the hatch cannot drift into an
/// allowlist. The gate validates both directions - a marked site that is not
/// recorded fails, and a recorded site without its marker fails (as does a
/// recorded site whose file no longer exists in the tree).
pub const HATCH_INVENTORY_PATH: &str = "packages/xtask/data/async-gate-inventory.json";

/// The hatch inventory file's shape: the marker format the scanner honors
/// (the prefix of the `// async-gate-allow: <reason>` trailing comment) and
/// the recorded sites.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HatchInventory {
    /// The source-level marker prefix, e.g. `// async-gate-allow:`.
    pub marker: String,
    /// Every marker-honored method-call lock site in the covered roots.
    pub sites: Vec<HatchSite>,
}

/// One recorded hatch site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HatchSite {
    /// Repository-relative path of the file, as the scanner reports it.
    pub file: String,
    /// 1-based line number of the marked method call.
    pub line: usize,
    /// Why the synchronous lock stays (the marker's `<reason>`).
    pub reason: String,
}

/// The default scan roots: the covered control-plane crates (broker, daemon,
/// daemon runtime, core, resource runtime) plus every provider crate (the
/// handler crates linked into the broker binary - a non-yielding handler
/// starves the whole envelope).
///
/// The resource-runtime/core/daemon-runtime roots are the shared substrate
/// converted by U4/U5/U17; they are scanned from day one so the gate cannot
/// regress while that conversion is in flight.
const DEFAULT_CRATE_ROOTS: &[&str] = &[
    "packages/d2b-broker",
    "packages/d2bd",
    "packages/d2bd-runtime",
    "packages/d2b-core",
    "packages/d2b-resource-runtime",
];

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

/// One scan's result: the violations found and every marker-honored site.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanOutcome {
    /// Blocking calls found on runtime workers.
    pub violations: Vec<Violation>,
    /// Every method-call lock site the hatch marker exempted - exactly the
    /// sites the inventory must record, with the marker's `<reason>`.
    pub marker_sites: Vec<HatchSite>,
}

/// One whole-run scan result: the per-file outcomes plus the scanned set.
struct ScanRun {
    violations: Vec<Violation>,
    marker_sites: Vec<HatchSite>,
    scanned_files: Vec<String>,
    file_count: usize,
}

/// A method-call lock shape whose fate (awaited or not) is not yet decided.
///
/// The call is pushed when `.lock(`/`.read(`/`.write(` is seen inside an
/// async context; `depth` counts the call's open parens. When the closing
/// paren lands, the call's legitimacy is decided by what follows it: `.await`
/// passes, end of line defers the decision to the next line, anything else
/// flags.
#[derive(Debug, Clone)]
struct PendingCall {
    /// 1-based line of the `.lock(`/`.read(`/`.write(`.
    line: usize,
    /// The method name (`lock`, `read`, `write`).
    method: String,
    /// `"async fn"` or `"async block"` - the async context the call sits in.
    context: &'static str,
    /// The offending line, trimmed.
    code: String,
    /// Open paren depth of the call; 0 when the call is closed.
    depth: usize,
}

impl PendingCall {
    fn into_violation(self, file: &str) -> Violation {
        Violation {
            file: file.to_owned(),
            line: self.line,
            api: format!(".{}()", self.method),
            context: self.context,
            code: self.code,
        }
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
}

/// Lexical scanner state across a file's lines.
struct ScanState {
    /// One entry per open brace level, innermost last.
    braces: Vec<BraceKind>,
    /// Whether the scanner is inside a `/* ... */` comment spanning lines.
    in_block_comment: bool,
    /// Hash count of an unterminated `r#"..."#` string spanning lines.
    raw_hashes: Option<usize>,
    /// The code since the last `;`, `{`, or `}` - used to recognize `async
    /// fn` signatures and the token before a `{` or `(`.
    segment: String,
    /// Method-call lock shapes whose closing paren has not landed yet,
    /// innermost last.
    open_calls: Vec<PendingCall>,
    /// A closed method-call lock shape waiting for the next non-space token
    /// to decide its fate: `.await` passes, anything else flags.
    await_pending: Option<PendingCall>,
}

impl ScanState {
    fn new() -> Self {
        Self {
            braces: Vec::new(),
            in_block_comment: false,
            raw_hashes: None,
            segment: String::new(),
            open_calls: Vec::new(),
            await_pending: None,
        }
    }
}

/// Scan one source file for blocking calls inside async contexts.
///
/// `marker` is the hatch marker prefix from the inventory (or `None` when the
/// hatch is not configured); a method-call lock shape on a line carrying the
/// marker is exempt and recorded in the outcome's `marker_sites`.
pub fn scan_source(
    file: &str,
    text: &str,
    entries: &[DeniedApi],
    marker: Option<&str>,
) -> ScanOutcome {
    let lock_methods = lock_method_names(entries);
    let mut state = ScanState::new();
    let mut outcome = ScanOutcome::default();
    for (index, line) in text.lines().enumerate() {
        // A line join is a token boundary: the sanitizer blanks each line
        // independently, and the scanner accumulates its segment across
        // lines, so without a separator `pub async` + newline + `fn` would
        // glue into one `asyncfn` identifier. Push a space at the join so
        // whitespace tokenisation splits `async` and `fn` apart.
        if index > 0 {
            state.segment.push(' ');
        }
        scan_line(
            &mut state,
            file,
            index + 1,
            line,
            entries,
            &lock_methods,
            marker,
            &mut outcome,
        );
    }
    outcome
}

/// The method names of the mutex/rwlock lock-acquisition entries on the deny
/// list (`std::sync`/`lock_api` `Mutex::lock`, `RwLock::read`,
/// `RwLock::write`): the conservative method-call shape the scanner flags
/// inside async contexts. Derived from the deny list so the list stays the
/// single source of truth - a new lock entry arms the gate automatically.
fn lock_method_names(entries: &[DeniedApi]) -> Vec<String> {
    let mut names = Vec::new();
    for entry in entries {
        let segments: Vec<&str> = entry.path.split("::").collect();
        if segments.len() >= 2
            && (segments[segments.len() - 2] == "Mutex"
                || segments[segments.len() - 2] == "RwLock")
        {
            let method = segments[segments.len() - 1].to_owned();
            if !names.contains(&method) {
                names.push(method);
            }
        }
    }
    names
}

/// Scan one line, advancing the scanner state and collecting violations.
#[allow(clippy::too_many_arguments)]
fn scan_line(
    state: &mut ScanState,
    file: &str,
    line_no: usize,
    raw: &str,
    entries: &[DeniedApi],
    lock_methods: &[String],
    marker: Option<&str>,
    outcome: &mut ScanOutcome,
) {
    let code = sanitize(raw, state);
    let bytes = code.as_bytes();
    let mut i = 0;
    let mut flagged: Vec<&str> = Vec::new();
    while i < bytes.len() {
        let b = bytes[i];
        let context = match state.braces.last() {
            Some(BraceKind::AsyncFn) => Some("async fn"),
            Some(BraceKind::AsyncBlock) => Some("async block"),
            _ => None,
        };
        // A closed method-call lock shape is decided by the next non-space
        // token (possibly on a later line): `.await` passes, anything else
        // flags.
        if let Some(pending) = state.await_pending.take() {
            if b.is_ascii_whitespace() {
                state.await_pending = Some(pending);
            } else if b == b'.' && starts_with_await(&code[i..]) {
                // Awaited tokio lock site: legitimate.
            } else {
                outcome.violations.push(pending.into_violation(file));
            }
        }
        match b {
            b'{' => {
                state.braces.push(brace_kind(state));
                state.segment.clear();
            }
            b'}' => {
                state.braces.pop();
                state.segment.clear();
            }
            b';' => {
                flush_pending(state, file, outcome);
                state.segment.clear();
            }
            b'(' => {
                if let Some(top) = state.open_calls.last_mut() {
                    top.depth += 1;
                }
                state.segment.push('(');
                i += 1;
                continue;
            }
            b')' => {
                if let Some(top) = state.open_calls.last_mut() {
                    top.depth -= 1;
                    if top.depth == 0 {
                        let pending = state.open_calls.pop().expect("top exists");
                        let rest = code[i + 1..].trim_start();
                        if starts_with_await(rest) {
                            // Awaited tokio lock site: legitimate.
                        } else if rest.is_empty() {
                            // The await decision lands on the next line.
                            state.await_pending = Some(pending);
                        } else {
                            outcome.violations.push(pending.into_violation(file));
                        }
                    }
                }
                state.segment.push(')');
                i += 1;
                continue;
            }
            b'.' => {
                if let Some(context) = context
                    && let Some(method) = method_call_lock_at(&code, i, lock_methods)
                {
                    if let Some(reason) = marker.and_then(|marker| marker_reason(raw, i, marker)) {
                        outcome.marker_sites.push(HatchSite {
                            file: file.to_owned(),
                            line: line_no,
                            reason,
                        });
                    } else {
                        state.open_calls.push(PendingCall {
                            line: line_no,
                            method: method.to_owned(),
                            context,
                            code: raw.trim().to_owned(),
                            depth: 0,
                        });
                    }
                }
                state.segment.push('.');
                i += 1;
                continue;
            }
            _ => {
                let token_start = i == 0 || !is_ident_char(bytes[i - 1]);
                if let Some(context) = context
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
                            outcome.violations.push(Violation {
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

/// Flag every pending method-call lock shape: a statement boundary (`;`)
/// cannot continue an un-awaited call, and an unclosed call is malformed
/// code the gate fails closed on.
fn flush_pending(state: &mut ScanState, file: &str, outcome: &mut ScanOutcome) {
    if let Some(pending) = state.await_pending.take() {
        outcome.violations.push(pending.into_violation(file));
    }
    while let Some(pending) = state.open_calls.pop() {
        outcome.violations.push(pending.into_violation(file));
    }
}

/// Whether a conservative method-call lock shape starts at byte position `i`
/// of the sanitized code: `.` followed by one of the lock method names and a
/// `(`, with a receiver expression before the `.` (skipping whitespace, and
/// accepting the line start for a method-chain continuation). Returns the
/// method name when it matches.
fn method_call_lock_at<'a>(code: &str, i: usize, lock_methods: &'a [String]) -> Option<&'a str> {
    let bytes = code.as_bytes();
    let mut j = i;
    while j > 0 && bytes[j - 1].is_ascii_whitespace() {
        j -= 1;
    }
    if j > 0 {
        let prev = bytes[j - 1];
        if !(is_ident_char(prev) || prev == b')' || prev == b']') {
            return None;
        }
    }
    for method in lock_methods {
        let after = i + 1;
        if code[after..].starts_with(method.as_str()) {
            let call = after + method.len();
            if bytes.get(call) == Some(&b'(') {
                return Some(method);
            }
        }
    }
    None
}

/// Whether `text` starts with the `.await` postfix at a token boundary:
/// `.await` followed by a non-identifier character (or end of text), so
/// `.awaitable()` never passes the await exclusion.
fn starts_with_await(text: &str) -> bool {
    let Some(rest) = text.strip_prefix(".await") else {
        return false;
    };
    rest.chars().next().is_none_or(|c| !is_ident_char(c as u8))
}

/// The hatch marker's `<reason>` when the marker appears on the raw line at
/// or after byte position `from` (the call's `.`): the text after the marker
/// prefix, trimmed (the documented form is
/// `// async-gate-allow: <reason>`). Sanitized positions align with raw
/// positions one-to-one, so `from` is valid in both. The marker is a
/// trailing comment on the call's own line; `None` when the marker is absent
/// or carries no reason.
fn marker_reason(raw: &str, from: usize, marker: &str) -> Option<String> {
    let trailing = trailing_line_comment_start(raw, from)?;
    let text = &raw[trailing..];
    let pos = text.find(marker)?;
    let after = trailing + pos + marker.len();
    let reason = raw[after..].trim();
    (!reason.is_empty()).then(|| reason.to_owned())
}

/// The byte offset of the trailing `//` line comment on the call's own line,
/// when one exists at or after `from` outside a string literal, char literal,
/// raw string, block comment, or line comment - the only comment text that
/// can carry the hatch marker (the sanitizer's comment handling: a `//` in
/// Normal mode runs to the end of the line, so the trailing comment is the
/// text after the *last* `//` outside any literal or block comment). String
/// literals, raw strings, and block comments never arm the hatch; raw strings
/// are tracked with the sanitizer's `raw_string_hashes`/`raw_string_closes`
/// rules, so an interior quote or a `//` inside raw-string content cannot
/// close the string early or arm the marker.
fn trailing_line_comment_start(raw: &str, from: usize) -> Option<usize> {
    let bytes = raw.as_bytes();
    let mut mode = Mode::Normal;
    let mut raw_hashes: Option<usize> = None;
    let mut i = from;
    let mut last: Option<usize> = None;
    while i < bytes.len() {
        let b = bytes[i];
        match mode {
            Mode::BlockComment => {
                if b == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    i += 2;
                    mode = Mode::Normal;
                } else {
                    i += 1;
                }
            }
            Mode::String => {
                if b == b'\\' {
                    i += 2;
                } else if b == b'"' {
                    i += 1;
                    mode = Mode::Normal;
                } else {
                    i += 1;
                }
            }
            Mode::Char => {
                if b == b'\\' {
                    i += 2;
                } else if b == b'\'' {
                    i += 1;
                    mode = Mode::Normal;
                } else {
                    i += 1;
                }
            }
            Mode::Normal => {
                if let Some(hashes) = raw_hashes {
                    // Inside a raw string only the exact `"` + `hashes` `#`s
                    // close delimiter ends it; quotes and `//` are content.
                    if b == b'"' && raw_string_closes(&bytes[i..], hashes) {
                        i += 1 + hashes;
                        raw_hashes = None;
                    } else {
                        i += 1;
                    }
                } else if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
                    last = Some(i);
                    // A `//` runs to the end of the line; everything after is
                    // comment text, so this is the trailing comment.
                    break;
                } else if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    i += 2;
                    mode = Mode::BlockComment;
                } else if b == b'"' {
                    if let Some(hashes) = raw_string_hashes(&bytes[..i]) {
                        raw_hashes = Some(hashes);
                        i += 1;
                    } else {
                        i += 1;
                        mode = Mode::String;
                    }
                } else if b == b'\'' {
                    if char_literal_starts(&bytes[i..]) {
                        i += 1;
                        mode = Mode::Char;
                    } else {
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
        }
    }
    last
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
    // The sanitizer already turns block comments into spaces, so splitting
    // the segment on whitespace recovers the token stream; arming on an
    // `async` token immediately followed by an `fn` token covers every
    // spelling rustc accepts (`pub async   fn`, `pub async` + newline +
    // `fn`, `pub async /* c */ fn`, including a hatch block between the
    // two) while `async fn_pointer()` and a lone `async` token stay
    // disarmed.
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    tokens.windows(2).any(|pair| pair == ["async", "fn"])
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
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn load_entries(repo_root: &Path) -> Result<Vec<DeniedApi>, String> {
    let clippy_toml = fs::read_to_string(repo_root.join("clippy.toml"))
        .map_err(|_| "async-gate: clippy.toml unreadable".to_owned())?;
    let entries = parse_deny_list(&clippy_toml);
    if entries.is_empty() {
        return Err("async-gate: no entries parsed from clippy.toml".to_owned());
    }
    Ok(entries)
}

/// Resolve [`DEFAULT_CRATE_ROOTS`] plus every provider crate under `packages`.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
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
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn scan_paths(
    repo_root: &Path,
    paths: &[PathBuf],
    entries: &[DeniedApi],
    marker: Option<&str>,
) -> Result<ScanRun, String> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            walk_rs(path, &mut files)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path.clone());
        }
    }
    files.sort();
    let mut run = ScanRun {
        violations: Vec::new(),
        marker_sites: Vec::new(),
        scanned_files: Vec::new(),
        file_count: files.len(),
    };
    for file in &files {
        let text = fs::read_to_string(file)
            .map_err(|error| format!("async-gate: read {}: {error}", file.display()))?;
        let display = file
            .strip_prefix(repo_root)
            .map(|relative| relative.display().to_string())
            .unwrap_or_else(|_| file.display().to_string());
        let outcome = scan_source(&display, &text, entries, marker);
        run.scanned_files.push(display);
        run.violations.extend(outcome.violations);
        run.marker_sites.extend(outcome.marker_sites);
    }
    Ok(run)
}

/// Load the hatch inventory - the marker format and the recorded sites.
/// The inventory is mandatory: the gate fails closed when it is missing or
/// malformed, so the hatch cannot be silently disabled.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn load_inventory(repo_root: &Path) -> Result<HatchInventory, String> {
    let path = repo_root.join(HATCH_INVENTORY_PATH);
    let text = fs::read_to_string(&path)
        .map_err(|_| format!("async-gate: hatch inventory unreadable at {}", path.display()))?;
    let inventory: HatchInventory = serde_json::from_str(&text)
        .map_err(|error| format!("async-gate: parse {}: {error}", path.display()))?;
    if inventory.marker.is_empty() {
        return Err("async-gate: hatch inventory marker format is empty".to_owned());
    }
    Ok(inventory)
}

/// Validate the hatch inventory against the scan, both directions:
///
/// * every marker-honored site must be recorded in the inventory,
/// * every recorded site must be a marker-honored site in a scanned file
///   (a stale entry - a marker removed - fails the gate), and
/// * every recorded site's file must still exist in the tree (a deleted or
///   moved marked file leaves a stale exemption in every scan mode).
///
/// Entries whose file exists but is outside the scanned set are ignored, so
/// a subset scan (`check-async-gate <paths>`) stays valid; the full
/// default-roots run validates every entry.
fn validate_inventory(
    repo_root: &Path,
    inventory: &HatchInventory,
    marker_sites: &[HatchSite],
    scanned_files: &[String],
) -> Result<(), String> {
    let recorded: BTreeSet<(&str, usize)> = inventory
        .sites
        .iter()
        .map(|site| (site.file.as_str(), site.line))
        .collect();
    let scanned: BTreeSet<&str> = scanned_files.iter().map(String::as_str).collect();
    let mut errors = Vec::new();
    for site in marker_sites {
        if !recorded.contains(&(site.file.as_str(), site.line)) {
            errors.push(format!(
                "{}:{}: marker-honored site is not recorded in {HATCH_INVENTORY_PATH}",
                site.file, site.line
            ));
        }
    }
    for site in &inventory.sites {
        if !repo_root.join(&site.file).is_file() {
            errors.push(format!(
                "{}:{}: inventory entry file does not exist in the tree",
                site.file, site.line
            ));
        } else if scanned.contains(site.file.as_str())
            && !marker_sites
                .iter()
                .any(|marked| marked.file == site.file && marked.line == site.line)
        {
            errors.push(format!(
                "{}:{}: inventory entry has no marker-honored method-call site",
                site.file, site.line
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "async-gate: hatch inventory drift ({}):\n{}",
            errors.len(),
            errors.join("\n")
        ))
    }
}

#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
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

/// Rewrite the hatch inventory from a run's marker sites: the regeneration
/// path for a line-shifting edit above a marked call (the ledger keys sites
/// by `(file, line)`, so any such edit turns the gate red until the entry is
/// re-recorded). Sites are sorted by `(file, line)` so the file is
/// byte-stable regardless of scan order, and two marked calls on one line
/// collapse to a single entry.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn write_inventory(
    repo_root: &Path,
    marker: &str,
    marker_sites: &[HatchSite],
) -> Result<(), String> {
    let mut sites = marker_sites.to_vec();
    sites.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    sites.dedup_by(|a, b| a.file == b.file && a.line == b.line);
    let inventory = HatchInventory {
        marker: marker.to_owned(),
        sites,
    };
    let path = repo_root.join(HATCH_INVENTORY_PATH);
    let json = serde_json::to_string_pretty(&inventory)
        .map_err(|error| format!("async-gate: serialize inventory: {error}"))?;
    fs::write(&path, json)
        .map_err(|error| format!("async-gate: write {}: {error}", path.display()))?;
    Ok(())
}

/// Run the gate: scan the default roots (or the given paths) and fail on any
/// blocking call inside an async context or any hatch inventory drift. With
/// `--write-inventory`, rewrite the inventory from the run's marker sites
/// instead of validating it - the regeneration path for a line-shifting edit
/// above a marked call.
pub fn run(repo_root: &Path, args: &[String]) -> Result<(), String> {
    let write_mode = args.iter().any(|arg| arg == "--write-inventory");
    // Reject any flag-shaped argument that is not the documented
    // `--write-inventory`: a mistyped flag (`--write-inventoryy`) used to
    // fall through to the path list and silently scan nothing.
    for arg in args {
        if arg.starts_with("--") && arg != "--write-inventory" {
            return Err(format!(
                "async-gate: unknown flag {arg:?} (the only flag is --write-inventory)"
            ));
        }
    }
    let paths_args: Vec<&String> = args
        .iter()
        .filter(|arg| *arg != "--write-inventory")
        .collect();
    if write_mode && !paths_args.is_empty() {
        return Err(
            "async-gate: --write-inventory requires the default scan roots (no <paths>): a subset scan would drop entries for unscanned files"
                .to_owned(),
        );
    }
    let entries = load_entries(repo_root)?;
    let inventory = load_inventory(repo_root)?;
    let paths = if paths_args.is_empty() {
        default_scan_paths(repo_root)?
    } else {
        paths_args.iter().map(PathBuf::from).collect()
    };
    // A resolved scan set that matches no `.rs` file (a mistyped or stale
    // path, an empty subtree) is a fatal scan error rather than a vacuous
    // pass: the gate must fail closed on "nothing scanned".
    if paths.is_empty() {
        return Err("async-gate: the resolved scan set is empty (no `.rs` file matched the given paths)".to_owned());
    }
    let run = scan_paths(repo_root, &paths, &entries, Some(&inventory.marker))?;
    // A resolved scan set with no `.rs` files is a mistake (a mistyped path,
    // a stray flag, an empty directory): failing the gate here stops a
    // vacuous "0 file(s) scanned, no blocking calls" success.
    if run.file_count == 0 {
        return Err(
            "async-gate: the resolved scan set is empty (no .rs files matched the scan paths): a zero-file scan must not report success"
                .to_owned(),
        );
    }
    if write_mode {
        write_inventory(repo_root, &inventory.marker, &run.marker_sites)?;
        println!(
            "async gate: regenerated {HATCH_INVENTORY_PATH} with {} site(s)",
            run.marker_sites.len()
        );
    } else {
        validate_inventory(repo_root, &inventory, &run.marker_sites, &run.scanned_files)?;
    }
    if run.violations.is_empty() {
        println!(
            "async gate: {} file(s) scanned, no blocking calls in async contexts",
            run.file_count
        );
        Ok(())
    } else {
        let rendered = run
            .violations
            .iter()
            .map(Violation::render)
            .collect::<Vec<_>>()
            .join("\n");
        Err(format!(
            "{} blocking call(s) in async context(s) across {} file(s):\n{rendered}",
            run.violations.len(),
            run.file_count
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deny_list() -> Vec<DeniedApi> {
        parse_deny_list(include_str!("../../../clippy.toml"))
    }

    /// Scan without the hatch configured.
    fn scan(file: &str, source: &str) -> Vec<Violation> {
        scan_source(file, source, &deny_list(), None).violations
    }

    /// Scan with the hatch marker configured.
    fn scan_hatched(file: &str, source: &str, marker: Option<&str>) -> ScanOutcome {
        scan_source(file, source, &deny_list(), marker)
    }

    const ERROR_FIXTURE: &str = "packages/d2b-broker/src/worker.rs";
    const MARKER: &str = "// async-gate-allow:";

    #[test]
    fn flags_blocking_calls_in_async_contexts() {
        let violations = scan(
            ERROR_FIXTURE,
            include_str!("../tests/fixtures/async-gate/blocking_in_async_fn.rs"),
        );
        let mut found: Vec<(&str, &str)> = violations
            .iter()
            .map(|violation| (violation.api.as_str(), violation.context))
            .collect();
        found.sort_unstable();
        // Each of the fixture's denied calls must be flagged exactly once in
        // its async context. Since U13 the `spawn_blocking` call and the
        // `std::fs::read` inside its argument body are both flagged (KD2: the
        // exemption is gone), so `std::fs::read` appears twice in `async fn`
        // contexts (load_config + via_spawn_blocking). The set comparison is
        // presence-per-API, not a whole-result equality: the deny list is the
        // single source of truth, and a whole-set equality here would pin
        // deny-list content instead of scanner behavior.
        for (api, context, expected) in [
            ("std::fs::read", "async fn", 2),
            ("std::fs::write", "async block", 1),
            ("std::sync::Mutex::lock", "async fn", 1),
            (".lock()", "async fn", 1),
            ("std::thread::sleep", "async fn", 1),
            ("tokio::task::spawn_blocking", "async fn", 1),
        ] {
            assert_eq!(
                found.iter().filter(|(a, c)| *a == api && *c == context).count(),
                expected,
                "{api} must be flagged {expected} time(s) in context {context}, found: {found:?}"
            );
        }
        for violation in &violations {
            assert_eq!(violation.file, ERROR_FIXTURE);
            assert!(!violation.code.is_empty());
        }
    }

    #[test]
    fn violations_render_as_named_json() {
        let violations = scan(
            ERROR_FIXTURE,
            include_str!("../tests/fixtures/async-gate/blocking_in_async_fn.rs"),
        );
        let rendered = violations[0].render();
        assert!(rendered.contains("\"violation\":\"blocking-call-in-async-context\""));
        assert!(rendered.contains("\"file\":\"packages/d2b-broker/src/worker.rs\""));
        assert!(rendered.contains("\"api\":\"std::fs::read\""));
        assert!(rendered.contains("\"context\":\"async fn\""));
    }

    #[test]
    fn passes_async_contexts_without_blocking_calls() {
        let violations = scan(
            "packages/d2b-broker/src/clean.rs",
            include_str!("../tests/fixtures/async-gate/clean_async.rs"),
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
        let violations = scan("x.rs", source);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn spawn_blocking_argument_bodies_are_flagged_like_async_code() {
        // U13 removed the `BraceKind::Blocking` exemption (the U3 split):
        // a `spawn_blocking` argument body is the caller's code and the call
        // itself is the banned thread-per-call shape, so the body's
        // `std::fs::read` and the `tokio::task::spawn_blocking` call must
        // both be flagged in the async fn.
        let source = "pub async fn load(path: &Path) -> Vec<u8> {\n    tokio::task::spawn_blocking(|| std::fs::read(path)).await.unwrap_or_default()\n}\n";
        let violations = scan("x.rs", source);
        assert!(
            violations
                .iter()
                .any(|violation| violation.api == "std::fs::read"),
            "a spawn_blocking argument body must flag: {violations:?}"
        );
        assert!(
            violations
                .iter()
                .any(|violation| violation.api == "tokio::task::spawn_blocking"),
            "the spawn_blocking call itself must flag: {violations:?}"
        );
    }

    #[test]
    fn comment_mentions_do_not_flag() {
        let source = "pub async fn load() {\n    // std::fs::read blocks; tokio::fs::read does not\n    tokio::fs::read(\"/x\").await.unwrap();\n}\n";
        let violations = scan("x.rs", source);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn tokio_replacements_do_not_flag() {
        let source = "pub async fn load(m: &tokio::sync::Mutex<u32>) {\n    tokio::fs::read(\"/x\").await.unwrap();\n    tokio::time::sleep(std::time::Duration::from_secs(1)).await;\n    tokio::sync::Mutex::lock(m).await;\n}\n";
        let violations = scan("x.rs", source);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn imported_module_calls_flag_via_the_tail() {
        let source = "use std::fs;\n\npub async fn load() {\n    fs::read(\"/x\").unwrap();\n}\n";
        let violations = scan("x.rs", source);
        assert!(
            violations.iter().any(|violation| violation.api == "std::fs::read"),
            "imported-module call must flag: {violations:?}"
        );
    }

    #[test]
    fn multi_line_async_fn_signatures_stay_async() {
        let source = "pub async fn load(\n    path: &Path,\n) -> Vec<u8> {\n    std::fs::read(path).unwrap_or_default()\n}\n";
        let violations = scan("x.rs", source);
        assert!(
            violations.iter().any(|violation| violation.api == "std::fs::read"),
            "a signature split across lines must still be an async context: {violations:?}"
        );
    }

    #[test]
    fn nested_blocks_inherit_the_async_context() {
        let source = "pub async fn load(path: &Path) {\n    if path.exists() {\n        std::fs::read(path).unwrap();\n    }\n}\n";
        let violations = scan("x.rs", source);
        assert!(
            violations.iter().any(|violation| violation.api == "std::fs::read"),
            "a block nested in an async fn still runs on the worker: {violations:?}"
        );
    }

    #[test]
    fn async_fn_pointer_does_not_open_an_async_context() {
        let source = "pub fn make() {\n    let g = async fn_pointer();\n    if g {\n        std::fs::read(\"/x\").unwrap();\n    }\n}\n";
        let violations = scan("x.rs", source);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn extended_roots_flag_async_contexts() {
        // A denied API inside an async fn in a newly-covered crate root is
        // flagged exactly like one in the broker or daemon: U3 puts
        // d2b-resource-runtime, d2b-core, and d2bd-runtime in the default
        // scan set, so a call in an async fn under one of those roots is a
        // violation (the substrate crates still convert in U4/U5/U17).
        let violations = scan(
            "packages/d2b-resource-runtime/src/watch.rs",
            include_str!("../tests/fixtures/async-gate/blocking_in_async_fn.rs"),
        );
        assert!(
            violations.iter().any(|violation| {
                violation.api == "std::fs::read" && violation.context == "async fn"
            }),
            "a denied call in an async fn under a covered root must flag: {:?}",
            violations.iter().map(Violation::render).collect::<Vec<_>>()
        );
    }

    #[test]
    fn default_scan_paths_cover_the_extended_roots() {
        let repo_root = {
            let mut path = std::env::current_dir().expect("current dir");
            loop {
                if path.join("Cargo.toml").is_file()
                    && path.join("BUILD.bazel").is_file()
                    && path.join("flake.nix").is_file()
                {
                    break path;
                }
                if !path.pop() {
                    panic!("cannot locate repo root");
                }
            }
        };
        let paths = default_scan_paths(&repo_root).expect("default scan paths resolve");
        for suffix in [
            "packages/d2b-broker",
            "packages/d2bd",
            "packages/d2b-resource-runtime",
            "packages/d2b-core",
            "packages/d2bd-runtime",
        ] {
            assert!(
                paths.iter().any(|path| path.ends_with(suffix)),
                "default scan roots must include {suffix}: {paths:?}"
            );
        }
    }

    #[test]
    fn worker_thread_recv_loops_are_not_flagged() {
        // R4's sanctioned shape: a blocking `sync_channel`/`mpsc` recv on the
        // worker's own thread (the loader_worker pattern). It is not an async
        // context, so even a denied API there must not flag.
        let source = "pub fn spawn_spec_writer() {\n    std::thread::spawn(move || {\n        loop {\n            let job = std::sync::mpsc::Receiver::recv(&rx).expect(\"writer gone\");\n            write_job(job);\n        }\n    });\n}\n";
        let violations = scan("x.rs", source);
        assert!(
            violations.is_empty(),
            "a worker thread's blocking recv is not an async context: {violations:?}"
        );
    }

    #[test]
    fn worker_recv_in_async_context_is_flagged() {
        // The same recv relocated onto a runtime worker is a violation: R4's
        // allowance is worker-thread-only, never an async context.
        let source = "pub async fn receive() {\n    std::sync::mpsc::Receiver::recv(&rx).unwrap();\n}\n";
        let violations = scan("x.rs", source);
        assert!(
            violations
                .iter()
                .any(|violation| violation.api == "std::sync::mpsc::Receiver::recv"),
            "a sync recv inside an async fn must flag: {violations:?}"
        );
    }

    // --- U6: method-call lock detection (issue #590, KTD4) ----------------

    #[test]
    fn method_call_lock_not_awaited_flags() {
        // The production lock shape: `m.lock()` on a std::sync mutex inside
        // an async fn, not followed by `.await`. The qualified-path deny
        // matching cannot see it; the conservative method-call shape must.
        let source = "pub async fn touch(m: &std::sync::Mutex<u32>) {\n    let _guard = m.lock().unwrap();\n}\n";
        let violations = scan("x.rs", source);
        assert!(
            violations
                .iter()
                .any(|violation| violation.api == ".lock()" && violation.context == "async fn"),
            "an un-awaited method-call lock inside an async fn must flag: {violations:?}"
        );
    }

    #[test]
    fn read_and_write_method_calls_flag() {
        // The RwLock acquisition family is part of the conservative shape:
        // `m.read()` and `m.write()` park the worker the same way.
        let source = "pub async fn touch(r: &std::sync::RwLock<u32>) {\n    let _a = r.read().unwrap();\n    let _b = r.write().unwrap();\n}\n";
        let violations = scan("x.rs", source);
        assert!(
            violations
                .iter()
                .any(|violation| violation.api == ".read()"),
            "an un-awaited method-call read must flag: {violations:?}"
        );
        assert!(
            violations
                .iter()
                .any(|violation| violation.api == ".write()"),
            "an un-awaited method-call write must flag: {violations:?}"
        );
    }

    #[test]
    fn awaited_method_call_lock_passes() {
        // The `.await` exclusion is load-bearing: an awaited tokio lock site
        // is legitimate (the census never sees it either).
        let source = "pub async fn touch(m: &tokio::sync::Mutex<u32>) {\n    let _guard = m.lock().await;\n}\n";
        let violations = scan("x.rs", source);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn awaited_method_call_lock_across_lines_passes() {
        // The await decision spans lines: a chain-formatted tokio lock site
        // must stay legitimate.
        let source = "pub async fn touch(m: &tokio::sync::Mutex<u32>) {\n    let _guard = m\n        .lock()\n        .await\n        .unwrap();\n}\n";
        let violations = scan("x.rs", source);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn await_lookalike_does_not_exempt() {
        // The await exclusion needs a token boundary: `.awaitable()` is not
        // `.await`, so the call must flag.
        let source = "pub async fn touch(m: &std::sync::Mutex<u32>) {\n    let _guard = m.lock()\n        .awaitable();\n}\n";
        let violations = scan("x.rs", source);
        assert!(
            violations
                .iter()
                .any(|violation| violation.api == ".lock()"),
            "an await-lookalike must not exempt the call: {violations:?}"
        );
    }

    #[test]
    fn unawaited_method_call_lock_across_lines_flags() {
        // The same chain form without the await is a violation, reported at
        // the call's own line.
        let source = "pub async fn touch(m: &std::sync::Mutex<u32>) {\n    let _guard = m\n        .lock()\n        .unwrap();\n}\n";
        let violations = scan("x.rs", source);
        assert!(
            violations
                .iter()
                .any(|violation| violation.api == ".lock()" && violation.line == 3),
            "an un-awaited chain-form lock must flag at its call line: {violations:?}"
        );
    }

    #[test]
    fn method_call_lock_in_sync_fn_passes() {
        // The conservative shape only applies inside async contexts: a
        // synchronous function is not a runtime worker.
        let source = "pub fn touch(m: &std::sync::Mutex<u32>) {\n    let _guard = m.lock().unwrap();\n}\n";
        let violations = scan("x.rs", source);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn method_call_lock_in_async_block_flags() {
        // An async block is a worker context like an async fn body.
        let source = "pub fn make() -> impl std::future::Future<Output = ()> {\n    async {\n        let _guard = m.lock().unwrap();\n    }\n}\n";
        let violations = scan("x.rs", source);
        assert!(
            violations
                .iter()
                .any(|violation| violation.api == ".lock()" && violation.context == "async block"),
            "an un-awaited method-call lock inside an async block must flag: {violations:?}"
        );
    }

    #[test]
    fn tokio_test_bodies_are_flagged_like_production() {
        // `#[tokio::test]` bodies run on a runtime, so a method-call lock
        // there is flagged like production code (and passes only with the
        // marker).
        let source = "#[tokio::test]\nasync fn touch() {\n    let _guard = m.lock().unwrap();\n}\n";
        let violations = scan("x.rs", source);
        assert!(
            violations
                .iter()
                .any(|violation| violation.api == ".lock()" && violation.context == "async fn"),
            "a method-call lock inside a tokio::test body must flag: {violations:?}"
        );
    }

    #[test]
    fn qualified_form_still_flags_with_marker_present() {
        // The marker exempts only the method-call shape: the qualified form
        // has no hatch and must keep failing.
        let source = "pub async fn touch(shared: &std::sync::Mutex<u32>) {\n    let _guard = std::sync::Mutex::lock(shared).unwrap(); // async-gate-allow: still forbidden\n}\n";
        let violations = scan("x.rs", source);
        assert!(
            violations
                .iter()
                .any(|violation| violation.api == "std::sync::Mutex::lock"),
            "the qualified form must keep flagging even with a marker: {violations:?}"
        );
    }

    #[test]
    fn marker_exempts_method_call_and_is_recorded() {
        // The documented hatch: a trailing `// async-gate-allow: <reason>`
        // comment on the call's own line exempts the site and records it for
        // the inventory.
        let source = "pub async fn touch(m: &std::sync::Mutex<u32>) {\n    let _guard = m.lock().unwrap(); // async-gate-allow: short critical section, no await inside\n}\n";
        let outcome = scan_hatched("x.rs", source, Some(MARKER));
        assert!(
            outcome.violations.is_empty(),
            "a marked method-call lock must pass: {:?}",
            outcome.violations.iter().map(Violation::render).collect::<Vec<_>>()
        );
        assert_eq!(
            outcome.marker_sites,
            vec![HatchSite {
                file: "x.rs".to_owned(),
                line: 2,
                reason: "short critical section, no await inside".to_owned(),
            }]
        );
    }

    #[test]
    fn marker_before_the_call_does_not_exempt() {
        // The marker must be on the call's own line at or after the call; a
        // marker on the line above is not the documented form.
        let source = "pub async fn touch(m: &std::sync::Mutex<u32>) {\n    // async-gate-allow: not the documented form\n    let _guard = m.lock().unwrap();\n}\n";
        let outcome = scan_hatched("x.rs", source, Some(MARKER));
        assert!(
            outcome
                .violations
                .iter()
                .any(|violation| violation.api == ".lock()"),
            "a marker on a different line must not exempt: {:?}",
            outcome.violations.iter().map(Violation::render).collect::<Vec<_>>()
        );
        assert!(outcome.marker_sites.is_empty());
    }

    #[test]
    fn marker_without_reason_does_not_exempt() {
        // The marker format requires a reason: `// async-gate-allow:` alone
        // is not the documented marker.
        let source = "pub async fn touch(m: &std::sync::Mutex<u32>) {\n    let _guard = m.lock().unwrap(); // async-gate-allow:\n}\n";
        let outcome = scan_hatched("x.rs", source, Some(MARKER));
        assert!(
            outcome
                .violations
                .iter()
                .any(|violation| violation.api == ".lock()"),
            "a marker without a reason must not exempt: {:?}",
            outcome.violations.iter().map(Violation::render).collect::<Vec<_>>()
        );
    }

    #[test]
    fn method_call_in_string_or_comment_does_not_flag() {
        // The sanitizer blanks strings and comments, so a `.lock()` mention
        // in prose or a literal is not a call.
        let source = "pub async fn touch() {\n    // m.lock() would park the worker\n    let s = \"m.lock() is the blocking shape\";\n    tokio::time::sleep(std::time::Duration::from_secs(1)).await;\n}\n";
        let violations = scan("x.rs", source);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn raw_string_comment_text_does_not_arm_the_hatch() {
        // A `//` inside raw-string content is not a comment: forged marker
        // text in a raw string must neither exempt the call nor be recorded.
        let source = r###"pub async fn touch(m: &std::sync::Mutex<u32>) {
    let _g = m.lock().unwrap(); let _s = r#"// async-gate-allow: forged"#;
}
"###;
        let outcome = scan_hatched("x.rs", source, Some(MARKER));
        assert!(
            outcome.violations.iter().any(|v| v.api == ".lock()"),
            "raw-string `//` text must not arm the hatch: {:?}",
            outcome
                .violations
                .iter()
                .map(Violation::render)
                .collect::<Vec<_>>()
        );
        assert!(outcome.marker_sites.is_empty(), "{outcome:?}");
    }

    #[test]
    fn raw_string_interior_quote_does_not_arm_the_hatch() {
        // The fail-open this pins: a bare quote inside a raw string used to
        // close the string early, so a `//` that is raw-string content was
        // accepted as the call's trailing comment and forged marker text
        // armed the hatch over a live un-awaited lock.
        let source = r###"pub async fn touch(m: &std::sync::Mutex<u32>) {
    let _g = m.lock().unwrap(); let _s = r#"say "// async-gate-allow: inside a raw string"#;
}
"###;
        let outcome = scan_hatched("x.rs", source, Some(MARKER));
        assert!(
            outcome.violations.iter().any(|v| v.api == ".lock()"),
            "an interior quote must not close a raw string early: {:?}",
            outcome
                .violations
                .iter()
                .map(Violation::render)
                .collect::<Vec<_>>()
        );
        assert!(outcome.marker_sites.is_empty(), "{outcome:?}");
    }

    #[test]
    fn multi_hash_raw_string_does_not_arm_the_hatch() {
        // The close delimiter is the quote plus exactly the opener's `#`s: a
        // `r##"..."##` string must not be closed by a bare `"`, and its
        // content must not arm the marker.
        let source = r###"pub async fn touch(m: &std::sync::Mutex<u32>) {
    let _g = m.lock().unwrap(); let _s = r##"say "// async-gate-allow: forged"##;
}
"###;
        let outcome = scan_hatched("x.rs", source, Some(MARKER));
        assert!(
            outcome.violations.iter().any(|v| v.api == ".lock()"),
            "a multi-hash raw string must not arm the hatch: {:?}",
            outcome
                .violations
                .iter()
                .map(Violation::render)
                .collect::<Vec<_>>()
        );
        assert!(outcome.marker_sites.is_empty(), "{outcome:?}");
    }

    #[test]
    fn byte_string_raw_does_not_arm_the_hatch() {
        // The `br#`/`cr#` prefixes are raw-string openers too; their content
        // is not comment text.
        let source = r###"pub async fn touch(m: &std::sync::Mutex<u32>) {
    let _g = m.lock().unwrap(); let _s = br#"say "// async-gate-allow: forged"#;
}
"###;
        let outcome = scan_hatched("x.rs", source, Some(MARKER));
        assert!(
            outcome.violations.iter().any(|v| v.api == ".lock()"),
            "a byte-string raw literal must not arm the hatch: {:?}",
            outcome
                .violations
                .iter()
                .map(Violation::render)
                .collect::<Vec<_>>()
        );
        assert!(outcome.marker_sites.is_empty(), "{outcome:?}");
    }

    #[test]
    fn marker_after_a_closed_raw_string_still_arms() {
        // The raw-string handling must not swallow a genuine trailing
        // comment that follows a closed raw string on the call's line.
        let source = r###"pub async fn touch(m: &std::sync::Mutex<u32>) {
    let _g = m.lock().unwrap(); let _s = r#"text"#; // async-gate-allow: short critical section
}
"###;
        let outcome = scan_hatched("x.rs", source, Some(MARKER));
        assert!(
            outcome.violations.is_empty(),
            "a real trailing marker after a closed raw string must arm: {:?}",
            outcome
                .violations
                .iter()
                .map(Violation::render)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            outcome.marker_sites,
            vec![site("x.rs", 2, "short critical section")]
        );
    }

    /// A throwaway repo root, removed on drop: a place to lay out the deny
    /// list, the hatch ledger, and scanned sources for `run()`-driven tests.
    static NEXT_TEMP_REPO: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    struct TempRepo {
        root: PathBuf,
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl TempRepo {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "async-gate-inventory-test-{}-{}",
                std::process::id(),
                NEXT_TEMP_REPO.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).expect("create temp root");
            Self { root }
        }

        /// A throwaway repo root containing one `.rs` file at `rel_path`, so
        /// inventory validation's tree-existence check sees it.
        fn with_file(rel_path: &str) -> Self {
            let repo = Self::new();
            repo.write(rel_path, "// placeholder body\n");
            repo
        }

        fn write(&self, rel_path: &str, content: &str) {
            let file = self.root.join(rel_path);
            fs::create_dir_all(file.parent().expect("rel path has a parent"))
                .expect("create temp tree");
            fs::write(&file, content).expect("write temp file");
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn site(file: &str, line: usize, reason: &str) -> HatchSite {
        HatchSite {
            file: file.to_owned(),
            line,
            reason: reason.to_owned(),
        }
    }

    #[test]
    fn inventory_validation_accepts_a_consistent_inventory() {
        let repo = TempRepo::with_file("packages/d2b-broker/src/runtime.rs");
        let inventory = HatchInventory {
            marker: MARKER.to_owned(),
            sites: vec![site("packages/d2b-broker/src/runtime.rs", 10, "short critical section")],
        };
        let marker_sites = vec![site("packages/d2b-broker/src/runtime.rs", 10, "short critical section")];
        let scanned = vec!["packages/d2b-broker/src/runtime.rs".to_owned()];
        assert!(validate_inventory(&repo.root, &inventory, &marker_sites, &scanned).is_ok());
    }

    #[test]
    fn inventory_validation_rejects_an_unrecorded_marker() {
        let repo = TempRepo::with_file("packages/d2b-broker/src/runtime.rs");
        let inventory = HatchInventory {
            marker: MARKER.to_owned(),
            sites: Vec::new(),
        };
        let marker_sites = vec![site("packages/d2b-broker/src/runtime.rs", 10, "short critical section")];
        let scanned = vec!["packages/d2b-broker/src/runtime.rs".to_owned()];
        let error = validate_inventory(&repo.root, &inventory, &marker_sites, &scanned)
            .expect_err("an unrecorded marker must fail");
        assert!(error.contains("not recorded"), "{error}");
    }

    #[test]
    fn inventory_validation_rejects_a_stale_entry() {
        let repo = TempRepo::with_file("packages/d2b-broker/src/worker.rs");
        let inventory = HatchInventory {
            marker: MARKER.to_owned(),
            sites: vec![site("packages/d2b-broker/src/worker.rs", 10, "short critical section")],
        };
        let marker_sites = Vec::new();
        let scanned = vec!["packages/d2b-broker/src/worker.rs".to_owned()];
        let error = validate_inventory(&repo.root, &inventory, &marker_sites, &scanned)
            .expect_err("a stale inventory entry must fail");
        assert!(error.contains("no marker-honored"), "{error}");
    }

    #[test]
    fn inventory_validation_ignores_entries_outside_the_scan() {
        // A subset scan (`check-async-gate <paths>`) must not fail on
        // inventory entries for files it did not scan - as long as the file
        // still exists in the tree.
        let repo = TempRepo::with_file("packages/d2b-broker/src/ops.rs");
        let inventory = HatchInventory {
            marker: MARKER.to_owned(),
            sites: vec![site("packages/d2b-broker/src/ops.rs", 10, "short critical section")],
        };
        let marker_sites = Vec::new();
        let scanned = vec!["packages/d2bd/src/composition.rs".to_owned()];
        assert!(validate_inventory(&repo.root, &inventory, &marker_sites, &scanned).is_ok());
    }

    #[test]
    fn inventory_validation_rejects_an_entry_whose_file_is_gone() {
        // A deleted or moved marked file leaves a stale exemption entry in
        // every scan mode: the file does not exist in the tree, so the entry
        // must fail even when the file is outside the scanned set.
        let repo = TempRepo::with_file("packages/d2b-broker/src/ops.rs");
        let inventory = HatchInventory {
            marker: MARKER.to_owned(),
            sites: vec![site(
                "packages/d2b-broker/src/deleted.rs",
                10,
                "short critical section",
            )],
        };
        let marker_sites = Vec::new();
        let scanned = vec!["packages/d2bd/src/composition.rs".to_owned()];
        let error = validate_inventory(&repo.root, &inventory, &marker_sites, &scanned)
            .expect_err("an entry for a missing file must fail");
        assert!(error.contains("does not exist in the tree"), "{error}");
    }

    /// The minimal deny list a `run()`-driven test repo needs: one
    /// `Mutex::lock` entry so the conservative method-call shape arms.
    const MINIMAL_CLIPPY_TOML: &str = "disallowed-methods = [\n    { path = \"std::sync::Mutex::lock\", reason = \"test deny list\", replacement = \"tokio::sync::Mutex::lock\" },\n]\n";

    /// A TempRepo laid out like the real repository for `run()`: the deny
    /// list, the hatch ledger at its committed path, and one marked source
    /// file under a default scan root. The committed ledger records the
    /// marked call at `committed_line`.
    fn gate_repo(source: &str, committed_line: usize) -> TempRepo {
        let repo = TempRepo::new();
        repo.write("clippy.toml", MINIMAL_CLIPPY_TOML);
        repo.write(
            HATCH_INVENTORY_PATH,
            &serde_json::to_string_pretty(&HatchInventory {
                marker: MARKER.to_owned(),
                sites: vec![site(
                    "packages/d2bd/src/composition.rs",
                    committed_line,
                    "short critical section",
                )],
            })
            .expect("serialize committed inventory"),
        );
        repo.write("packages/d2bd/src/composition.rs", source);
        repo
    }

    /// A TempRepo with the deny list and an empty ledger but no scan roots.
    fn empty_gate_repo() -> TempRepo {
        let repo = TempRepo::new();
        repo.write("clippy.toml", MINIMAL_CLIPPY_TOML);
        repo.write(
            HATCH_INVENTORY_PATH,
            &serde_json::to_string_pretty(&HatchInventory {
                marker: MARKER.to_owned(),
                sites: Vec::new(),
            })
            .expect("serialize committed inventory"),
        );
        repo
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn read_inventory(repo_root: &Path) -> HatchInventory {
        let text = fs::read_to_string(repo_root.join(HATCH_INVENTORY_PATH))
            .expect("read the ledger back");
        serde_json::from_str(&text).expect("parse the ledger back")
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn write_inventory_regenerates_a_shifted_marked_line() {
        // Drive the real regeneration path: `run()` with `--write-inventory`
        // rewrites the ledger from the run's marker sites, so a line-shifting
        // edit above a marked call is re-recorded and the gate passes again
        // without hand-editing the ledger.
        let source = "pub async fn touch(m: &std::sync::Mutex<u32>) {\n    // a line pushed in above\n    let _guard = m.lock().unwrap(); // async-gate-allow: short critical section\n}\n";
        // The committed ledger still records the pre-shift line 2.
        let repo = gate_repo(source, 2);
        let error = run(&repo.root, &[]).expect_err("the committed ledger must be stale");
        assert!(error.contains("hatch inventory drift"), "{error}");
        run(&repo.root, &["--write-inventory".to_owned()])
            .expect("regeneration must succeed");
        let inventory = read_inventory(&repo.root);
        assert_eq!(
            inventory.sites,
            vec![site(
                "packages/d2bd/src/composition.rs",
                3,
                "short critical section"
            )],
            "the ledger must re-record the shifted site"
        );
        // The regenerated ledger is committed: a plain validation run passes.
        run(&repo.root, &[]).expect("the regenerated ledger must validate");
        // Regeneration is byte-stable: a second run rewrites identical bytes.
        let before = fs::read(repo.root.join(HATCH_INVENTORY_PATH)).expect("read the ledger");
        run(&repo.root, &["--write-inventory".to_owned()])
            .expect("a second regeneration must succeed");
        let after = fs::read(repo.root.join(HATCH_INVENTORY_PATH)).expect("read the ledger");
        assert_eq!(before, after, "regeneration must be byte-stable");
    }

    #[test]
    fn write_inventory_sorts_and_dedups_sites() {
        // The ledger writer must emit sites sorted by (file, line) with
        // same-line duplicates collapsed, so the file is byte-stable
        // regardless of scan order.
        let repo = TempRepo::new();
        // The ledger's parent directory exists in the real tree; a bare temp
        // repo needs it created before `write_inventory` can write.
        repo.write("packages/xtask/data/async-gate-inventory.json", "{}");
        let sites = vec![
            site("packages/d2bd/src/ops.rs", 10, "second"),
            site("packages/d2bd/src/composition.rs", 3, "first"),
            site("packages/d2bd/src/composition.rs", 10, "third"),
            site("packages/d2bd/src/composition.rs", 10, "duplicate"),
        ];
        write_inventory(&repo.root, MARKER, &sites).expect("write the ledger");
        let inventory = read_inventory(&repo.root);
        assert_eq!(inventory.marker, MARKER);
        assert_eq!(
            inventory.sites,
            vec![
                site("packages/d2bd/src/composition.rs", 3, "first"),
                site("packages/d2bd/src/composition.rs", 10, "third"),
                site("packages/d2bd/src/ops.rs", 10, "second"),
            ]
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn write_inventory_refuses_a_subset_scan_and_leaves_the_ledger_untouched() {
        // `--write-inventory` over explicit paths would drop entries for
        // unscanned files, so the mode refuses subset scans and must not
        // touch the committed ledger.
        let repo = gate_repo(
            "pub async fn touch(m: &std::sync::Mutex<u32>) {\n    let _guard = m.lock().unwrap(); // async-gate-allow: short critical section\n}\n",
            2,
        );
        let before = fs::read(repo.root.join(HATCH_INVENTORY_PATH)).expect("read the ledger");
        let error = run(
            &repo.root,
            &[
                "--write-inventory".to_owned(),
                "packages/d2bd/src/composition.rs".to_owned(),
            ],
        )
        .expect_err("a subset regeneration must fail");
        assert!(error.contains("requires the default scan roots"), "{error}");
        let after = fs::read(repo.root.join(HATCH_INVENTORY_PATH)).expect("read the ledger");
        assert_eq!(
            before, after,
            "a refused regeneration must leave the ledger untouched"
        );
    }

    #[test]
    fn run_rejects_an_unknown_flag() {
        // A mistyped flag used to fall through to the path list and silently
        // scan nothing; the gate must fail closed.
        let repo = gate_repo(
            "pub async fn touch(m: &std::sync::Mutex<u32>) {\n    let _guard = m.lock().unwrap(); // async-gate-allow: short critical section\n}\n",
            2,
        );
        let error = run(&repo.root, &["--write-inventoryy".to_owned()])
            .expect_err("an unknown flag must fail");
        assert!(error.contains("unknown flag"), "{error}");
    }

    #[test]
    fn run_fails_closed_on_an_empty_resolved_scan_set() {
        // A repo whose default roots resolve to nothing (only the ledger's
        // `packages` tree exists) must fail rather than pass vacuously.
        let repo = empty_gate_repo();
        let error = run(&repo.root, &[]).expect_err("an empty scan set must fail");
        assert!(error.contains("resolved scan set is empty"), "{error}");
    }

    #[test]
    fn run_fails_closed_on_a_zero_file_scan() {
        // A scan set that resolves to directories but no `.rs` file must not
        // report a vacuous success.
        let repo = empty_gate_repo();
        repo.write("packages/d2bd/src/README.txt", "not rust\n");
        let error = run(&repo.root, &[]).expect_err("a zero-file scan must fail");
        assert!(error.contains("zero-file scan"), "{error}");
    }

    #[test]
    fn lock_method_names_derive_from_the_deny_list() {
        let entries = deny_list();
        let names = lock_method_names(&entries);
        assert!(names.contains(&"lock".to_owned()));
        assert!(names.contains(&"read".to_owned()));
        assert!(names.contains(&"write".to_owned()));
        // The io trait methods are not lock acquisitions and must not arm the
        // conservative shape.
        assert!(!names.contains(&"open".to_owned()));
        assert!(!names.contains(&"recv".to_owned()));
    }
}
