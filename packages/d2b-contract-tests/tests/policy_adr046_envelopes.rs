//! ADR-046 resource-envelope structural lints.
//!
//! Two ADR-046 decisions have, until now, had no mechanical enforcement and
//! have each survived multiple manual sweeps: D116 (a Host or Guest that admits
//! the `user` domain must name a `defaultUserRef`) and the universal status
//! contract (D088 three-layer status base + D091 `status.update` currency
//! object). Manual sweeps keep missing siblings in files never previously
//! cited; a lint does not.
//!
//! Both scanners parse every fenced YAML/JSON/Nix block into a real document
//! model and then assert over the parsed structure. An earlier generation of
//! these lints matched line shapes with regexes; that made them fail open in
//! several ways (JSON envelopes were never read at all, a missing frame key
//! silently skipped a whole document, a bare `...` anywhere in the status
//! subtree skipped the document, and an inline `status: {}` was accepted). The
//! root cause was line-oriented heuristics over structured formats. This file
//! parses instead, and it fails **closed**: a block that clearly intends a
//! resource envelope (it carries both an `apiVersion` key and a type field) but
//! that the parser cannot model is reported, never skipped.
//!
//! The parser is deliberately small and covers the block-mapping subset the
//! ADR-046 specs actually use: JSON and Nix attribute sets share a brace
//! tokenizer; YAML is parsed by indentation. Comments (`#`, `//`, `/* */`) are
//! stripped before structural parsing so a commented-out key can never satisfy
//! a rule, and flow collections (`[ ... ]`, `{ ... }`) embedded in YAML values
//! are delegated to the brace parser.
//!
//! Distinctions the grammar draws:
//!
//! * A **live resource envelope** carries an `apiVersion` key and a `type`
//!   scalar. When it also shows a `status`, that status must carry both
//!   `update` (D091 currency object) and `resource` (D088 Layer-2 base) as
//!   direct children, unless the status body is deliberately abbreviated with a
//!   `...` elision that is a **direct child of `status`**. An inline
//!   `status: {}` or `status: null` on a live envelope is incomplete, not
//!   abbreviated, and is flagged.
//! * A **bundle envelope** carries an `apiVersion` key and a `resourceType`
//!   scalar in place of `type`. These are emitted by the Nix resource compiler
//!   and deliberately carry `status: null` (the status is a runtime concern);
//!   see `docs/specs/ADR-046-resources-zone-control.md`. They are a distinct
//!   contract and are not required to carry a status base.
//! * A **focused fragment** shows only part of an object - a lone `status:`
//!   snippet with no `apiVersion`, or a Host `spec` with no `allowedDomains`.
//!   A fragment carries no `apiVersion` key and so is never treated as an
//!   envelope.
//! * A **shorthand schema table** is a Markdown `| ... |` table describing a
//!   schema in prose. Tables are never inside a code fence, so neither scanner
//!   ever reads one.
//!
//! Each scanner is exercised by planted-violation fixtures - one per closed
//! bypass - and clean-input fixtures, then run over the real tree. A stricter
//! lint that surfaces a real `docs/**` violation is the correct outcome, never
//! a reason to weaken the grammar.

use std::path::{Path, PathBuf};

use d2b_contract_tests::repo_root;

/// A single violation: the repo-relative file, 1-based line number, and a
/// human-actionable description of what is missing or malformed.
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

// ---------------------------------------------------------------------------
// Fenced-code-block model.
//
// Both scanners read only fenced code blocks; prose and Markdown tables are
// never a resource-authoring context. A block carries its language tag (so each
// scanner can restrict to the formats it parses) and the absolute 0-based index
// of its first body line (so a violation can be reported at a real file line).
// ---------------------------------------------------------------------------

struct Block<'a> {
    lang: String,
    body_start: usize,
    lines: Vec<&'a str>,
}

fn fenced_blocks(content: &str) -> Vec<Block<'_>> {
    let lines: Vec<&str> = content.lines().collect();
    let mut blocks = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        let trimmed = lines[idx].trim_start();
        if let Some(rest) = trimmed.strip_prefix("```") {
            let lang = rest.trim().to_string();
            let body_start = idx + 1;
            let mut end = body_start;
            while end < lines.len() && !lines[end].trim_start().starts_with("```") {
                end += 1;
            }
            blocks.push(Block {
                lang,
                body_start,
                lines: lines[body_start..end.min(lines.len())].to_vec(),
            });
            idx = end + 1;
        } else {
            idx += 1;
        }
    }
    blocks
}

/// The count of leading ASCII spaces on `line`.
fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

// ---------------------------------------------------------------------------
// Structural document model.
//
// A `Node` is the parsed shape of a fenced block. `Map` preserves author order
// and the block-relative line of each key so a violation can be reported at a
// real file line. `Elision` is the `...` / `"..."` abbreviation marker the
// specs use to say "more fields, deliberately not shown here".
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Map(Vec<Entry>),
    Seq(Vec<Node>),
    Scalar(String),
    Null,
    Elision,
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    key: String,
    val: Node,
    /// Block-relative 0-based line index of the key.
    line: usize,
}

/// A direct child of a mapping, by key. No recursion: this is used where the
/// contract is about a map's own direct children (status base keys, type).
fn direct_child<'a>(map: &'a [Entry], key: &str) -> Option<&'a Entry> {
    map.iter().find(|e| e.key == key)
}

/// Collect every mapping in the document tree (the node itself and every
/// descendant), so a scanner can locate every envelope regardless of nesting.
fn collect_maps<'a>(node: &'a Node, out: &mut Vec<&'a [Entry]>) {
    match node {
        Node::Map(entries) => {
            out.push(entries.as_slice());
            for e in entries {
                collect_maps(&e.val, out);
            }
        }
        Node::Seq(items) => {
            for it in items {
                collect_maps(it, out);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Comment stripping.
// ---------------------------------------------------------------------------

/// Strip `#`, `//`, and `/* */` comments from brace-structured text (JSON, Nix,
/// and YAML flow scalars), preserving newlines so token line numbers stay
/// aligned with the source. Comment characters inside a double-quoted string
/// are preserved.
fn strip_comments_brace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_str = false;
    let mut in_line = false;
    let mut in_block = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if in_line {
            if c == '\n' {
                in_line = false;
                out.push(c);
            }
            continue;
        }
        if in_block {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block = false;
            } else if c == '\n' {
                out.push('\n');
            }
            continue;
        }
        if in_str {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            '#' => in_line = true,
            '/' if chars.peek() == Some(&'/') => {
                chars.next();
                in_line = true;
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                in_block = true;
            }
            _ => out.push(c),
        }
    }
    out
}

/// Strip a trailing YAML `#` comment from a single line, respecting quotes and
/// the YAML rule that `#` starts a comment only at line start or after
/// whitespace.
fn strip_yaml_comment(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut in_s = false;
    let mut in_d = false;
    let mut prev_ws = true;
    for (idx, &b) in bytes.iter().enumerate() {
        match b {
            b'\'' if !in_d => {
                in_s = !in_s;
                prev_ws = false;
            }
            b'"' if !in_s => {
                in_d = !in_d;
                prev_ws = false;
            }
            b'#' if !in_s && !in_d && prev_ws => {
                return line[..idx].to_string();
            }
            b' ' | b'\t' => prev_ws = true,
            _ => prev_ws = false,
        }
    }
    line.to_string()
}

// ---------------------------------------------------------------------------
// Brace tokenizer + parser (JSON, Nix attribute sets, YAML flow collections).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum TokKind {
    LBrace,
    RBrace,
    LBrack,
    RBrack,
    Colon,
    Sep,
    Str(String),
    Word(String),
}

struct Tok {
    kind: TokKind,
    line: usize,
}

fn tokenize_brace(text: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut line = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => line += 1,
            c if c.is_whitespace() => {}
            '{' => toks.push(Tok {
                kind: TokKind::LBrace,
                line,
            }),
            '}' => toks.push(Tok {
                kind: TokKind::RBrace,
                line,
            }),
            '[' => toks.push(Tok {
                kind: TokKind::LBrack,
                line,
            }),
            ']' => toks.push(Tok {
                kind: TokKind::RBrack,
                line,
            }),
            ':' | '=' => toks.push(Tok {
                kind: TokKind::Colon,
                line,
            }),
            ',' | ';' => toks.push(Tok {
                kind: TokKind::Sep,
                line,
            }),
            '"' => {
                let mut s = String::new();
                let mut escaped = false;
                for ch in chars.by_ref() {
                    if escaped {
                        s.push(ch);
                        escaped = false;
                    } else if ch == '\\' {
                        escaped = true;
                    } else if ch == '"' {
                        break;
                    } else {
                        if ch == '\n' {
                            line += 1;
                        }
                        s.push(ch);
                    }
                }
                toks.push(Tok {
                    kind: TokKind::Str(s),
                    line,
                });
            }
            _ => {
                let mut w = String::new();
                w.push(c);
                while let Some(&nc) = chars.peek() {
                    if nc.is_whitespace()
                        || matches!(nc, '{' | '}' | '[' | ']' | ':' | '=' | ',' | ';' | '"')
                    {
                        break;
                    }
                    w.push(nc);
                    chars.next();
                }
                toks.push(Tok {
                    kind: TokKind::Word(w),
                    line,
                });
            }
        }
    }
    toks
}

fn scalar_or_special(s: String) -> Node {
    if s == "..." {
        Node::Elision
    } else if s == "null" || s == "~" {
        Node::Null
    } else {
        Node::Scalar(s)
    }
}

fn parse_brace_block(lines: &[&str]) -> Vec<Node> {
    let text = strip_comments_brace(&lines.join("\n"));
    let toks = tokenize_brace(&text);
    let mut cur = 0;
    while cur < toks.len() && matches!(toks[cur].kind, TokKind::Sep) {
        cur += 1;
    }
    if cur >= toks.len() {
        return vec![Node::Map(Vec::new())];
    }
    let node = match toks[cur].kind {
        TokKind::LBrace | TokKind::LBrack => parse_brace_value(&toks, &mut cur),
        _ => Node::Map(parse_brace_map_body(&toks, &mut cur, false)),
    };
    vec![node]
}

fn parse_brace_value(toks: &[Tok], cur: &mut usize) -> Node {
    if *cur >= toks.len() {
        return Node::Null;
    }
    match &toks[*cur].kind {
        TokKind::LBrace => {
            *cur += 1;
            Node::Map(parse_brace_map_body(toks, cur, true))
        }
        TokKind::LBrack => {
            *cur += 1;
            Node::Seq(parse_brace_seq(toks, cur))
        }
        TokKind::Str(s) | TokKind::Word(s) => {
            let v = s.clone();
            *cur += 1;
            scalar_or_special(v)
        }
        _ => {
            *cur += 1;
            Node::Null
        }
    }
}

fn parse_brace_map_body(toks: &[Tok], cur: &mut usize, expect_rbrace: bool) -> Vec<Entry> {
    let mut entries = Vec::new();
    loop {
        while *cur < toks.len() && matches!(toks[*cur].kind, TokKind::Sep) {
            *cur += 1;
        }
        if *cur >= toks.len() {
            break;
        }
        match &toks[*cur].kind {
            TokKind::RBrace => {
                if expect_rbrace {
                    *cur += 1;
                }
                break;
            }
            TokKind::RBrack => break,
            TokKind::LBrace | TokKind::LBrack => {
                let _ = parse_brace_value(toks, cur);
            }
            TokKind::Colon | TokKind::Sep => *cur += 1,
            TokKind::Str(s) | TokKind::Word(s) => {
                let key = s.clone();
                let kline = toks[*cur].line;
                *cur += 1;
                if key == "..." {
                    // Consume a `"...": "..."` form fully; a bare `"..."` has no
                    // colon. Either way it is an elision marker.
                    if *cur < toks.len() && matches!(toks[*cur].kind, TokKind::Colon) {
                        *cur += 1;
                        let _ = parse_brace_value(toks, cur);
                    }
                    entries.push(Entry {
                        key,
                        val: Node::Elision,
                        line: kline,
                    });
                    continue;
                }
                if *cur < toks.len() && matches!(toks[*cur].kind, TokKind::Colon) {
                    *cur += 1;
                    let val = parse_brace_value(toks, cur);
                    entries.push(Entry {
                        key,
                        val,
                        line: kline,
                    });
                } else {
                    entries.push(Entry {
                        key,
                        val: Node::Null,
                        line: kline,
                    });
                }
            }
        }
    }
    entries
}

fn parse_brace_seq(toks: &[Tok], cur: &mut usize) -> Vec<Node> {
    let mut items = Vec::new();
    loop {
        while *cur < toks.len() && matches!(toks[*cur].kind, TokKind::Sep) {
            *cur += 1;
        }
        if *cur >= toks.len() {
            break;
        }
        if matches!(toks[*cur].kind, TokKind::RBrack) {
            *cur += 1;
            break;
        }
        if matches!(toks[*cur].kind, TokKind::RBrace) {
            break;
        }
        items.push(parse_brace_value(toks, cur));
    }
    items
}

// ---------------------------------------------------------------------------
// YAML indentation parser.
// ---------------------------------------------------------------------------

/// Index of the first `:` in `t` that separates a mapping key from its value:
/// not inside quotes, and followed by a space or end-of-line.
fn find_yaml_colon(t: &str) -> Option<usize> {
    let bytes = t.as_bytes();
    let mut in_s = false;
    let mut in_d = false;
    for (idx, &b) in bytes.iter().enumerate() {
        match b {
            b'\'' if !in_d => in_s = !in_s,
            b'"' if !in_s => in_d = !in_d,
            b':' if !in_s && !in_d => {
                let next = bytes.get(idx + 1);
                if next.is_none() || next == Some(&b' ') {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_yaml_scalar_or_flow(s: &str) -> Node {
    let st = s.trim();
    if st.starts_with('{') || st.starts_with('[') {
        let text = strip_comments_brace(st);
        let toks = tokenize_brace(&text);
        let mut cur = 0;
        return parse_brace_value(&toks, &mut cur);
    }
    let unq = st.trim_matches('"').trim_matches('\'');
    if unq == "..." {
        Node::Elision
    } else if unq == "null" || unq == "~" || unq.is_empty() {
        Node::Null
    } else {
        Node::Scalar(unq.to_string())
    }
}

/// Indentation of the next non-blank, non-`---` line at or after `i`.
fn peek_next_indent(refs: &[&str], i: usize) -> Option<usize> {
    let mut j = i;
    while j < refs.len() {
        let t = refs[j].trim();
        if t.is_empty() {
            j += 1;
            continue;
        }
        if t == "---" {
            return None;
        }
        return Some(indent(refs[j]));
    }
    None
}

/// Whether the next content line at exactly `indent_level` is a sequence item.
fn next_is_sequence(refs: &[&str], i: usize, indent_level: usize) -> bool {
    let mut j = i;
    while j < refs.len() {
        let t = refs[j].trim();
        if t.is_empty() {
            j += 1;
            continue;
        }
        if indent(refs[j]) != indent_level {
            return false;
        }
        return t == "-" || t.starts_with("- ");
    }
    false
}

fn parse_yaml_child(refs: &[&str], i: &mut usize, child_indent: usize) -> Node {
    if next_is_sequence(refs, *i, child_indent) {
        Node::Seq(parse_yaml_sequence(refs, i, child_indent))
    } else {
        Node::Map(parse_yaml_mapping(refs, i, child_indent))
    }
}

fn parse_yaml_mapping(refs: &[&str], i: &mut usize, indent_level: usize) -> Vec<Entry> {
    let mut entries = Vec::new();
    while *i < refs.len() {
        let raw = refs[*i];
        let t = raw.trim();
        if t.is_empty() {
            *i += 1;
            continue;
        }
        if t == "---" {
            break;
        }
        let ind = indent(raw);
        if ind < indent_level {
            break;
        }
        if ind > indent_level {
            *i += 1;
            continue;
        }
        if t == "..." {
            entries.push(Entry {
                key: "...".to_string(),
                val: Node::Elision,
                line: *i,
            });
            *i += 1;
            continue;
        }
        if t == "-" || t.starts_with("- ") {
            // A sequence at this level is not a mapping; hand back to the caller.
            break;
        }
        let Some(colon) = find_yaml_colon(t) else {
            *i += 1;
            continue;
        };
        let key = t[..colon]
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        let after = t[colon + 1..].trim();
        let kline = *i;
        *i += 1;
        if after.is_empty() {
            match peek_next_indent(refs, *i) {
                Some(ci) if ci > indent_level => {
                    let val = parse_yaml_child(refs, i, ci);
                    entries.push(Entry {
                        key,
                        val,
                        line: kline,
                    });
                }
                _ => entries.push(Entry {
                    key,
                    val: Node::Null,
                    line: kline,
                }),
            }
        } else {
            entries.push(Entry {
                key,
                val: parse_yaml_scalar_or_flow(after),
                line: kline,
            });
        }
    }
    entries
}

fn parse_yaml_sequence(refs: &[&str], i: &mut usize, indent_level: usize) -> Vec<Node> {
    let mut items = Vec::new();
    while *i < refs.len() {
        let raw = refs[*i];
        let t = raw.trim();
        if t.is_empty() {
            *i += 1;
            continue;
        }
        if t == "---" {
            break;
        }
        let ind = indent(raw);
        if ind < indent_level {
            break;
        }
        if ind > indent_level {
            *i += 1;
            continue;
        }
        let rest = if t == "-" {
            Some("")
        } else {
            t.strip_prefix("- ")
        };
        let Some(rest) = rest else {
            break;
        };
        let rest = rest.trim();
        if rest.is_empty() {
            *i += 1;
            match peek_next_indent(refs, *i) {
                Some(ci) if ci > indent_level => items.push(parse_yaml_child(refs, i, ci)),
                _ => items.push(Node::Null),
            }
        } else if let Some(colon) = find_yaml_colon(rest) {
            // Inline first key of a map item; further keys sit one level deeper
            // than the dash (dash column + 2).
            let key = rest[..colon].trim().trim_matches('"').to_string();
            let after = rest[colon + 1..].trim();
            let kline = *i;
            let mut item_entries = Vec::new();
            *i += 1;
            if after.is_empty() {
                match peek_next_indent(refs, *i) {
                    Some(ci) if ci > ind + 1 => {
                        let val = parse_yaml_child(refs, i, ci);
                        item_entries.push(Entry {
                            key,
                            val,
                            line: kline,
                        });
                    }
                    _ => item_entries.push(Entry {
                        key,
                        val: Node::Null,
                        line: kline,
                    }),
                }
            } else {
                item_entries.push(Entry {
                    key,
                    val: parse_yaml_scalar_or_flow(after),
                    line: kline,
                });
            }
            let more = parse_yaml_mapping(refs, i, ind + 2);
            item_entries.extend(more);
            items.push(Node::Map(item_entries));
        } else {
            items.push(parse_yaml_scalar_or_flow(rest));
            *i += 1;
        }
    }
    items
}

fn parse_yaml_block(lines: &[&str]) -> Vec<Node> {
    let stripped: Vec<String> = lines.iter().map(|l| strip_yaml_comment(l)).collect();
    let refs: Vec<&str> = stripped.iter().map(|s| s.as_str()).collect();
    let mut docs = Vec::new();
    let mut i = 0;
    while i < refs.len() {
        while i < refs.len() {
            let t = refs[i].trim();
            if t.is_empty() || t == "---" {
                i += 1;
            } else {
                break;
            }
        }
        if i >= refs.len() {
            break;
        }
        let base = indent(refs[i]);
        let node = if next_is_sequence(&refs, i, base) {
            Node::Seq(parse_yaml_sequence(&refs, &mut i, base))
        } else {
            Node::Map(parse_yaml_mapping(&refs, &mut i, base))
        };
        docs.push(node);
    }
    docs
}

/// Parse a fenced block into one or more top-level documents. JSON and Nix use
/// the brace parser; YAML uses the indentation parser; an untagged fence is
/// sniffed by its first non-comment character.
fn parse_block_docs(lang: &str, lines: &[&str]) -> Vec<Node> {
    match lang {
        "json" | "nix" => parse_brace_block(lines),
        "yaml" | "yml" => parse_yaml_block(lines),
        "" => {
            let stripped = strip_comments_brace(&lines.join("\n"));
            match stripped.trim_start().chars().next() {
                Some('{') | Some('[') => parse_brace_block(lines),
                _ => parse_yaml_block(lines),
            }
        }
        _ => Vec::new(),
    }
}

/// Whether a line carries `key` (optionally quoted) followed by `:` or `=`,
/// anywhere on the line. Used only to detect that a block clearly intends an
/// envelope so the scanner can fail closed if the parser cannot model it.
fn mentions_key(lines: &[&str], key: &str) -> bool {
    lines.iter().any(|l| {
        let mut hay = *l;
        while let Some(pos) = hay.find(key) {
            let after = &hay[pos + key.len()..];
            let after = after.trim_start_matches('"').trim_start();
            if after.starts_with(':') || after.starts_with('=') {
                return true;
            }
            hay = &hay[pos + key.len()..];
        }
        false
    })
}

// ---------------------------------------------------------------------------
// D116 - a Host or Guest admitting the `user` domain must name a defaultUserRef.
//
// The decision register (D116) freezes the superset invariant: `defaultUserRef`
// is present whenever `allowedDomains` contains `user`, independent of the
// default domain. A `defaultUserRef` that is present but `null` does not satisfy
// it - a null ref is the absence of a ref. The grammar is language-neutral so a
// YAML, JSON, or Nix authoring block is judged identically, and because the
// block is parsed structurally a multiline `allowedDomains` list is read the
// same as an inline one and a commented-out `defaultUserRef` cannot satisfy the
// invariant.
//
// A block is judged only when it declares a Host/Guest resource that carries an
// `allowedDomains` list - i.e. an execution-policy authoring context. A block
// that shows a Host spec without `allowedDomains`, or `allowedDomains` without a
// Host/Guest `type`, is a focused fragment and is not judged.
//
// A spec may also show an *intentional negative example* - an authored shape
// meant to be rejected, e.g. the block that demonstrates the D116 eval-time
// failure by deliberately omitting `defaultUserRef`. That exemption is pinned to
// exactly one file and one marker: it applies only in
// `docs/specs/ADR-046-nix-configuration.md`, only to a block carrying the exact
// comment `d2b-lint: expect-d116-eval-error`, and only when that marker occurs
// exactly once in the file. The same marker in any other file, or a duplicated
// marker, does not suppress anything - it fails closed.
// ---------------------------------------------------------------------------

const D116_EXEMPT_FILE: &str = "docs/specs/ADR-046-nix-configuration.md";
const D116_EXEMPT_MARKER: &str = "d2b-lint: expect-d116-eval-error";

/// Whether `line` is exactly the pinned D116 negative-example marker comment.
/// Only a comment line (`#` or `//`) carrying exactly the marker text qualifies,
/// so a stray mention in prose or a string value cannot suppress a violation.
fn is_d116_marker_line(line: &str) -> bool {
    let t = line.trim();
    let t = t.trim_start_matches(['#', '/']).trim();
    t == D116_EXEMPT_MARKER
}

/// The Host/Guest type of a mapping, if it declares one as a direct child.
fn host_guest_type(map: &[Entry]) -> Option<String> {
    for key in ["type", "resourceType"] {
        if let Some(Node::Scalar(s)) = direct_child(map, key).map(|e| &e.val)
            && (s == "Host" || s == "Guest")
        {
            return Some(s.clone());
        }
    }
    None
}

/// A resource field looked up as a direct child of the resource map, or one
/// level down under its `spec`. Scoped to the given map, so one document's field
/// can never satisfy another.
fn resource_entry<'a>(map: &'a [Entry], key: &str) -> Option<&'a Entry> {
    if let Some(e) = direct_child(map, key) {
        return Some(e);
    }
    if let Some(Node::Map(inner)) = direct_child(map, "spec").map(|e| &e.val) {
        return direct_child(inner, key);
    }
    None
}

fn seq_has_scalar(node: &Node, want: &str) -> bool {
    match node {
        Node::Seq(items) => items
            .iter()
            .any(|it| matches!(it, Node::Scalar(s) if s == want)),
        _ => false,
    }
}

/// Whether a `defaultUserRef` value is a real reference: a present, non-empty,
/// non-null scalar.
fn ref_is_set(node: Option<&Node>) -> bool {
    matches!(node, Some(Node::Scalar(s)) if !s.is_empty() && s != "null")
}

fn scan_d116(file: &str, content: &str) -> Vec<Violation> {
    let marker_count = content.lines().filter(|l| is_d116_marker_line(l)).count();
    let mut out = Vec::new();
    for block in fenced_blocks(content) {
        if !matches!(block.lang.as_str(), "yaml" | "yml" | "json" | "nix" | "") {
            continue;
        }
        let exempt = file == D116_EXEMPT_FILE
            && marker_count == 1
            && block.lines.iter().any(|l| is_d116_marker_line(l));
        for doc in parse_block_docs(&block.lang, &block.lines) {
            let mut maps = Vec::new();
            collect_maps(&doc, &mut maps);
            for m in &maps {
                if host_guest_type(m).is_none() {
                    continue;
                }
                let Some(ad) = resource_entry(m, "allowedDomains") else {
                    continue;
                };
                if !seq_has_scalar(&ad.val, "user") {
                    continue;
                }
                if ref_is_set(resource_entry(m, "defaultUserRef").map(|e| &e.val)) {
                    continue;
                }
                if exempt {
                    continue;
                }
                out.push(Violation {
                    file: file.to_string(),
                    line: block.body_start + ad.line + 1,
                    text: "Host/Guest with `user` in allowedDomains is missing a non-null `defaultUserRef` (D116)"
                        .to_string(),
                });
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Universal status contract - every live envelope carries the D088 base plus
// `status.update` (D091) and `status.resource` (D088 Layer 2).
//
// A live envelope is any fenced YAML/JSON document that carries an `apiVersion`
// key and a `type` scalar. When it also shows a `status`, that status subtree
// must carry both `update` and `resource` as direct children. A subtree that
// carries `credential`, `device`, or any other ResourceType-specific key in
// place of `resource` is a violation: the Layer-2 key is frozen as `resource`.
//
// Elision is honoured narrowly: only a `...` (or `"...": "..."`) elision that is
// a direct child of `status` abbreviates the status body. A `...` nested deeper
// (inside `conditions`, `update`, etc.) does not, and an inline `status: {}` or
// `status: null` on a live envelope is incomplete rather than abbreviated.
//
// A bundle envelope (an `apiVersion` document whose type field is `resourceType`
// rather than `type`) is the compiler-emitted contract that deliberately carries
// `status: null`; it is recognized as distinct and is not required to carry a
// status base.
//
// The scanner fails closed: a block that carries both an `apiVersion` key and a
// type field but that the parser cannot model into an envelope is reported, not
// skipped.
// ---------------------------------------------------------------------------

fn status_violation(file: &str, line: usize, missing: &[&str]) -> Violation {
    Violation {
        file: file.to_string(),
        line,
        text: format!(
            "complete resource envelope is missing {} (D088/D091 universal status base)",
            missing.join(" and ")
        ),
    }
}

fn scan_universal_status(file: &str, content: &str) -> Vec<Violation> {
    let mut out = Vec::new();
    for block in fenced_blocks(content) {
        if !matches!(block.lang.as_str(), "yaml" | "yml" | "json" | "") {
            continue;
        }
        if !mentions_key(&block.lines, "apiVersion") {
            continue;
        }
        let mut found_envelope = false;
        for doc in parse_block_docs(&block.lang, &block.lines) {
            let mut maps = Vec::new();
            collect_maps(&doc, &mut maps);
            for m in &maps {
                if direct_child(m, "apiVersion").is_none() {
                    continue;
                }
                let has_type = direct_child(m, "type").is_some();
                let has_resource_type = direct_child(m, "resourceType").is_some();
                if !has_type && !has_resource_type {
                    continue;
                }
                found_envelope = true;
                // A bundle envelope (resourceType, no type) is the compiler
                // contract with a deliberately null status: a distinct contract.
                if !has_type && has_resource_type {
                    continue;
                }
                let Some(status) = direct_child(m, "status") else {
                    continue; // spec-only fragment: no status shown.
                };
                let line = block.body_start + status.line + 1;
                match &status.val {
                    // status: ... (inline elision) is a deliberate abbreviation.
                    Node::Elision => {}
                    // status: null / status: {} on a live envelope is incomplete.
                    Node::Null => out.push(status_violation(
                        file,
                        line,
                        &["status.update", "status.resource"],
                    )),
                    Node::Map(entries) => {
                        let elided = entries
                            .iter()
                            .any(|e| e.key == "..." || matches!(e.val, Node::Elision));
                        if elided {
                            continue;
                        }
                        let mut missing = Vec::new();
                        if !entries.iter().any(|e| e.key == "update") {
                            missing.push("status.update");
                        }
                        if !entries.iter().any(|e| e.key == "resource") {
                            missing.push("status.resource");
                        }
                        if !missing.is_empty() {
                            out.push(status_violation(file, line, &missing));
                        }
                    }
                    Node::Scalar(_) | Node::Seq(_) => out.push(status_violation(
                        file,
                        line,
                        &["status.update", "status.resource"],
                    )),
                }
            }
        }
        // Fail closed: a block that clearly intends an envelope (an apiVersion
        // key plus a type field) but that produced no envelope map is a parser
        // gap, not a pass.
        if !found_envelope
            && mentions_key(&block.lines, "apiVersion")
            && (mentions_key(&block.lines, "type") || mentions_key(&block.lines, "resourceType"))
        {
            out.push(Violation {
                file: file.to_string(),
                line: block.body_start + 1,
                text: "block declares an `apiVersion` resource envelope the structural parser could not model; a lint must fail closed on an unparseable envelope, not skip it (D088/D091)"
                    .to_string(),
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Real-tree enumeration.
// ---------------------------------------------------------------------------

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
        "{kind}: {} envelope-structure violation(s) under docs/specs/**:\n",
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
// D116 fixtures.
// ---------------------------------------------------------------------------

#[test]
fn d116_flags_a_host_or_guest_admitting_user_without_a_default_user_ref() {
    // YAML Host with user domain and no ref.
    let yaml_missing = "\
```yaml
apiVersion: resources.d2bus.org/v3
type: Host
spec:
  allowedDomains: [system, user]
```";
    assert_eq!(scan_d116("f.md", yaml_missing).len(), 1);

    // A present-but-null ref does not satisfy D116.
    let yaml_null = "\
```yaml
type: Host
spec:
  allowedDomains: [system, user]
  defaultUserRef: null
```";
    assert_eq!(scan_d116("f.md", yaml_null).len(), 1);

    // Nix Guest with user domain and no ref.
    let nix_missing = "\
```nix
resources.x = {
  type = \"Guest\";
  spec = { allowedDomains = [ \"system\" \"user\" ]; };
};
```";
    assert_eq!(scan_d116("f.md", nix_missing).len(), 1);
}

#[test]
fn d116_flags_a_multiline_allowed_domains_list_without_a_ref() {
    // BYPASS CLOSED: a multiline YAML `allowedDomains` sequence was previously
    // missed because only the key line was inspected for brackets. It is now
    // parsed as a real list.
    let multiline = "\
```yaml
type: Host
spec:
  allowedDomains:
    - system
    - user
```";
    assert_eq!(
        scan_d116("f.md", multiline).len(),
        1,
        "multiline allowedDomains list without a ref must be flagged"
    );

    // The same multiline list WITH a ref is clean.
    let multiline_ok = "\
```yaml
type: Host
spec:
  allowedDomains:
    - system
    - user
  defaultUserRef: User/alice
```";
    assert!(scan_d116("f.md", multiline_ok).is_empty());
}

#[test]
fn d116_ignores_a_commented_out_default_user_ref() {
    // BYPASS CLOSED: a commented-out `defaultUserRef` must not satisfy the
    // invariant. Comments are stripped before structural parsing.
    let commented = "\
```yaml
type: Host
spec:
  allowedDomains: [system, user]
  # defaultUserRef: User/alice
```";
    assert_eq!(
        scan_d116("f.md", commented).len(),
        1,
        "a commented-out defaultUserRef must not satisfy D116"
    );
}

#[test]
fn d116_does_not_let_one_document_satisfy_another_in_the_same_fence() {
    // BYPASS CLOSED: two documents in one fence were previously evaluated as a
    // single object, so a ref in one satisfied the other. They are now parsed
    // as separate documents.
    let two_docs = "\
```yaml
type: Host
spec:
  allowedDomains: [system, user]
  defaultUserRef: User/alice
---
type: Guest
spec:
  allowedDomains: [system, user]
```";
    assert_eq!(
        scan_d116("f.md", two_docs).len(),
        1,
        "the second document's missing ref must still be flagged"
    );
}

#[test]
fn d116_accepts_clean_shapes() {
    // A real ref satisfies the invariant (YAML, JSON single-line, and Nix).
    for ok in [
        "```yaml\ntype: Host\nspec:\n  allowedDomains: [system, user]\n  defaultUserRef: User/alice\n```",
        "```json\n{ \"type\": \"Host\", \"spec\": { \"allowedDomains\": [\"system\", \"user\"], \"defaultUserRef\": \"User/alice\" } }\n```",
        "```json\n{\n  \"spec\": { \"defaultDomain\": \"system\", \"allowedDomains\": [\"system\", \"user\"], \"defaultUserRef\": \"User/alice\" },\n  \"type\": \"Host\"\n}\n```",
        "```nix\nx = { type = \"Guest\"; spec = { allowedDomains = [ \"system\" \"user\" ]; defaultUserRef = \"User/alice\"; }; };\n```",
    ] {
        assert!(scan_d116("f.md", ok).is_empty(), "clean Host/Guest: {ok:?}");
    }

    // A Host that does not admit the user domain needs no ref.
    let system_only = "```yaml\ntype: Host\nspec:\n  allowedDomains: [system]\n```";
    assert!(scan_d116("f.md", system_only).is_empty());

    // A focused fragment: allowedDomains with no Host/Guest type declaration.
    let no_type = "```yaml\nspec:\n  allowedDomains: [system, user]\n```";
    assert!(scan_d116("f.md", no_type).is_empty());

    // A focused fragment: a Host spec that does not show allowedDomains.
    let no_domains = "```yaml\ntype: Host\nspec:\n  providerRef: Provider/system-core\n```";
    assert!(scan_d116("f.md", no_domains).is_empty());

    // A Markdown schema table (never inside a fence) is not scanned.
    let table = "| field | rule |\n| type: Host | allowedDomains [system, user] |";
    assert!(scan_d116("f.md", table).is_empty());
}

#[test]
fn d116_negative_example_marker_is_pinned_to_one_file_and_block() {
    // The intentional negative example (the D116 eval-error teaching block) is
    // exempt only in the pinned file, only when it carries the exact marker.
    let marked = "\
```nix
d2b.zones.dev.resources.host-system = {
  type = \"Host\";
  spec = {
    allowedDomains = [\"system\" \"user\"];
    # defaultUserRef intentionally omitted -> eval error
    # d2b-lint: expect-d116-eval-error
  };
};
```";
    assert!(
        scan_d116(D116_EXEMPT_FILE, marked).is_empty(),
        "marked negative example in the pinned file is exempt"
    );

    // BYPASS CLOSED: the SAME marker in ANY OTHER file does not suppress. The
    // exemption was previously unbounded (any comment mentioning d2b-lint and
    // d116, anywhere).
    assert_eq!(
        scan_d116(
            "docs/specs/ADR-046-resources-host-guest-process-user.md",
            marked
        )
        .len(),
        1,
        "the marker must not suppress a violation outside the pinned file"
    );

    // BYPASS CLOSED: a DUPLICATED marker in the pinned file fails closed - the
    // exemption is for exactly one deliberate block, not a family of them.
    let duplicated = "\
```nix
d2b.zones.dev.resources.host-a = {
  type = \"Host\";
  spec = {
    allowedDomains = [\"system\" \"user\"];
    # d2b-lint: expect-d116-eval-error
  };
};
```
```nix
d2b.zones.dev.resources.host-b = {
  type = \"Host\";
  spec = {
    allowedDomains = [\"system\" \"user\"];
    # d2b-lint: expect-d116-eval-error
  };
};
```";
    assert_eq!(
        scan_d116(D116_EXEMPT_FILE, duplicated).len(),
        2,
        "a duplicated marker suppresses nothing (fail closed)"
    );

    // The pinned shape WITHOUT the marker is still a violation even in the
    // pinned file: a real declaration never carries the self-incriminating
    // comment, so genuine misses stay flagged.
    let unmarked = "\
```nix
d2b.zones.dev.resources.host-system = {
  type = \"Host\";
  spec = {
    allowedDomains = [\"system\" \"user\"];
    # defaultUserRef intentionally omitted -> eval error
  };
};
```";
    assert_eq!(
        scan_d116(D116_EXEMPT_FILE, unmarked).len(),
        1,
        "unmarked stays flagged"
    );
}

// ---------------------------------------------------------------------------
// Universal-status fixtures.
// ---------------------------------------------------------------------------

const COMPLETE_STATUS: &str = "  observedGeneration: 1
  phase: Ready
  conditions: []
  update:
    state: Current
  resource:
    availability: ready";

fn envelope_with_status(status_body: &str) -> String {
    format!(
        "```yaml\napiVersion: resources.d2bus.org/v3\ntype: Widget\nmetadata:\n  name: x\nspec:\n  providerRef: Provider/p\nstatus:\n{status_body}\n```"
    )
}

#[test]
fn universal_status_flags_a_complete_envelope_missing_update_or_resource() {
    // Missing status.update.
    let no_update = envelope_with_status(
        "  observedGeneration: 1\n  phase: Ready\n  resource:\n    availability: ready",
    );
    let v = scan_universal_status("f.md", &no_update);
    assert_eq!(v.len(), 1, "{}", report("dbg", &v));
    assert!(v[0].text.contains("status.update"));

    // A ResourceType-specific key in place of `resource` is still a violation:
    // both status.update and status.resource are missing.
    let credential_key = envelope_with_status(
        "  observedGeneration: 1\n  phase: Ready\n  credential:\n    leaseState: Active",
    );
    let v = scan_universal_status("f.md", &credential_key);
    assert_eq!(v.len(), 1);
    assert!(v[0].text.contains("status.update"));
    assert!(v[0].text.contains("status.resource"));
}

#[test]
fn universal_status_reads_json_envelopes() {
    // BYPASS CLOSED: a JSON resource envelope was previously never checked
    // because the scanner accepted only `yaml`/`yml` fences. A JSON envelope
    // missing status.resource must now be flagged.
    let json_missing_resource = "\
```json
{
  \"apiVersion\": \"resources.d2bus.org/v3\",
  \"type\": \"Widget\",
  \"metadata\": { \"name\": \"x\" },
  \"spec\": { \"providerRef\": \"Provider/p\" },
  \"status\": {
    \"phase\": \"Ready\",
    \"update\": { \"state\": \"Current\" }
  }
}
```";
    let v = scan_universal_status("f.md", json_missing_resource);
    assert_eq!(v.len(), 1, "{}", report("dbg", &v));
    assert!(v[0].text.contains("status.resource"));

    // A complete JSON envelope carrying both keys is clean.
    let json_ok = "\
```json
{
  \"apiVersion\": \"resources.d2bus.org/v3\",
  \"type\": \"Widget\",
  \"status\": {
    \"phase\": \"Ready\",
    \"update\": { \"state\": \"Current\" },
    \"resource\": { \"availability\": \"ready\" }
  }
}
```";
    assert!(
        scan_universal_status("f.md", json_ok).is_empty(),
        "{}",
        report("dbg", &scan_universal_status("f.md", json_ok))
    );
}

#[test]
fn universal_status_checks_a_document_missing_a_frame_key() {
    // BYPASS CLOSED: omitting a frame key (here `metadata` and `spec`) used to
    // make the whole document silently unchecked. A document that carries
    // apiVersion + type + a concrete status is now checked regardless.
    let missing_frame = "\
```yaml
apiVersion: resources.d2bus.org/v3
type: Widget
status:
  phase: Ready
  update:
    state: Current
```";
    let v = scan_universal_status("f.md", missing_frame);
    assert_eq!(v.len(), 1, "{}", report("dbg", &v));
    assert!(v[0].text.contains("status.resource"));
}

#[test]
fn universal_status_only_honours_elision_as_a_direct_child_of_status() {
    // BYPASS CLOSED: a bare `...` nested BELOW status (here inside `conditions`)
    // used to skip the whole document. It no longer does; the status base is
    // still required.
    let nested_elision =
        envelope_with_status("  phase: Ready\n  conditions:\n    - reason: X\n      ...");
    let v = scan_universal_status("f.md", &nested_elision);
    assert_eq!(
        v.len(),
        1,
        "a `...` nested below status must not exempt the document: {}",
        report("dbg", &v)
    );
    assert!(v[0].text.contains("status.update"));
    assert!(v[0].text.contains("status.resource"));

    // A `...` that IS a direct child of status is a deliberate abbreviation.
    let direct_elision = envelope_with_status("  phase: Ready\n  ...");
    assert!(
        scan_universal_status("f.md", &direct_elision).is_empty(),
        "a `...` direct child of status abbreviates the body"
    );
}

#[test]
fn universal_status_rejects_inline_empty_and_null_status_on_live_envelopes() {
    // BYPASS CLOSED: an inline `status: {}` was accepted. On a live (type)
    // envelope it is incomplete, not abbreviated.
    let empty = "```json\n{ \"apiVersion\": \"resources.d2bus.org/v3\", \"type\": \"Widget\", \"status\": {} }\n```";
    let v = scan_universal_status("f.md", empty);
    assert_eq!(
        v.len(),
        1,
        "inline status: {{}} must be flagged: {}",
        report("dbg", &v)
    );

    // A `status: null` on a live envelope is likewise incomplete.
    let null_status =
        "```yaml\napiVersion: resources.d2bus.org/v3\ntype: Widget\nstatus: null\n```";
    assert_eq!(
        scan_universal_status("f.md", null_status).len(),
        1,
        "status: null on a live envelope must be flagged"
    );
}

#[test]
fn universal_status_treats_bundle_envelopes_as_a_distinct_contract() {
    // A bundle envelope uses `resourceType` (not `type`) and carries a
    // deliberately null status; it is the compiler-emitted contract and must
    // NOT be flagged for a missing status base.
    let bundle = "\
```json
{
  \"apiVersion\": \"resources.d2bus.org/v3\",
  \"resourceType\": \"Role\",
  \"metadata\": { \"name\": \"process-controller\", \"zone\": \"dev\" },
  \"spec\": { \"rules\": [] },
  \"status\": null
}
```";
    assert!(
        scan_universal_status("f.md", bundle).is_empty(),
        "{}",
        report("dbg", &scan_universal_status("f.md", bundle))
    );

    // But a LIVE envelope (type) with status: null IS flagged - the distinction
    // is the type field, not the null status.
    let live_null = "\
```json
{
  \"apiVersion\": \"resources.d2bus.org/v3\",
  \"type\": \"Role\",
  \"status\": null
}
```";
    assert_eq!(
        scan_universal_status("f.md", live_null).len(),
        1,
        "a live envelope with status: null must be flagged"
    );
}

#[test]
fn universal_status_accepts_fragment_and_abbreviated_shapes() {
    // A complete envelope carrying both keys is clean.
    let clean = envelope_with_status(COMPLETE_STATUS);
    assert!(
        scan_universal_status("f.md", &clean).is_empty(),
        "{}",
        report("dbg", &scan_universal_status("f.md", &clean))
    );

    // A focused fragment: a lone status subtree with no apiVersion frame.
    let fragment = "```yaml\nstatus:\n  observedGeneration: 1\n  phase: Ready\n```";
    assert!(scan_universal_status("f.md", fragment).is_empty());

    // A spec-only envelope (no status) is not a complete-status context.
    let spec_only = "```yaml\napiVersion: resources.d2bus.org/v3\ntype: Widget\nmetadata:\n  name: x\nspec:\n  providerRef: Provider/p\n```";
    assert!(scan_universal_status("f.md", spec_only).is_empty());
}

#[test]
fn universal_status_ignores_prose_field_path_references() {
    // Legitimate explanatory prose references a status field path under the
    // spec's documented `status.<field>` -> `status.resource.<field>` mapping
    // convention. These are correct content, not resource-envelope examples,
    // and MUST NOT be flagged: the scanner only reads fenced documents, and
    // prose is never inside a fence.
    let prose = "\
Each credential surfaces its expiry as `Credential.status.credential.expiresAtUnixMs`,
which the provider maps onto `status.resource.expiresAtUnixMs` per the universal
status base. The envelope carries apiVersion, type, metadata, spec and status,
and its status.update block records the reconcile generation.

A caller reads Credential.status.credential.leaseState the same way.";
    assert!(
        scan_universal_status("prose.md", prose).is_empty(),
        "{}",
        report("dbg", &scan_universal_status("prose.md", prose))
    );

    // The same prose alongside a genuinely correct fenced envelope: the fence
    // is scanned and passes, and the prose reference is still ignored.
    let mixed = format!(
        "The `status.credential` field maps to `status.resource` in examples.\n\n{}\n\nSee `Credential.status.credential.expiresAtUnixMs` above.",
        envelope_with_status(COMPLETE_STATUS)
    );
    assert!(
        scan_universal_status("mixed.md", &mixed).is_empty(),
        "{}",
        report("dbg", &scan_universal_status("mixed.md", &mixed))
    );

    // A shorthand schema table using `status.credential` column text is prose,
    // not a fenced envelope, so it is never scanned.
    let table = "| field | maps to |\n| status.credential.expiresAtUnixMs | status.resource.expiresAtUnixMs |";
    assert!(scan_universal_status("table.md", table).is_empty());
}

// ---------------------------------------------------------------------------
// Real-tree gates. These scan the committed docs/specs/** tree; a stricter lint
// surfacing a real docs violation is the correct outcome, and the failure names
// every offending block for the author to fix.
// ---------------------------------------------------------------------------

#[test]
fn docs_specs_host_guest_declare_a_default_user_ref_for_user_domain() {
    let violations = scan_spec_tree(scan_d116);
    assert!(
        violations.is_empty(),
        "{}",
        report("D116 (defaultUserRef)", &violations)
    );
}

#[test]
fn docs_specs_complete_envelopes_carry_the_universal_status_base() {
    let violations = scan_spec_tree(scan_universal_status);
    assert!(
        violations.is_empty(),
        "{}",
        report("universal status (D088/D091)", &violations)
    );
}
