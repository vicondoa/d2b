//! Shared structural document model for the ADR-046 policy lints.
//!
//! The earlier generation of these lints shipped a hand-written parser that
//! tried to model JSON, YAML, and Nix with a single brace tokenizer plus a
//! YAML-by-indentation reader. Three independent reviewers concluded that a
//! hand-written parser for three languages cannot be trusted to model them
//! correctly: JSON `\uXXXX` escapes were never decoded, valid Nix constructs
//! (`rec`, `let`) were parsed then discarded, YAML anchors/merge keys were
//! silently mishandled, and there was no explicit parse-error channel so a
//! block the parser could not model was silently treated as "nothing to
//! check" (fail open).
//!
//! This module replaces that parser with real parsers - `serde_json` for JSON,
//! `serde_yaml_ng` for YAML, and `rnix` for Nix - behind one small `Node`
//! model. Every parse has an explicit error channel (`Result<_, ParseError>`)
//! and every caller treats an error as **fail closed**, never as "nothing to
//! check".
//!
//! Documented schematic conventions are recognized as first-class and
//! normalized so they parse into distinct `Node` variants rather than being
//! silently skipped:
//!
//! * `...` (elision) - "more fields, deliberately not shown" - parses to
//!   [`Node::Elision`].
//! * `<placeholder>` - an authored schema placeholder such as `<name>` or
//!   `<ResourceType>` - parses to [`Node::Placeholder`].
//! * Nix string interpolation and non-literal scalar expressions parse to
//!   [`Node::Opaque`]: present, but not a concrete literal a value-position
//!   check can inspect. Structural expressions that may hide an attribute set
//!   either expose their body to the walker or return [`ParseError`]; they never
//!   silently collapse a whole document to `Opaque`.
//!
//! Semantic keys are normalized: a JSON `"ty\u0070e"` key and a bare YAML
//! `type:` key both resolve to the key `type`, because the real parsers decode
//! escapes before this module ever sees the key.

#![allow(dead_code)]

use rnix::{SyntaxKind, SyntaxNode};
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use std::fmt;
use std::path::Path;

/// A parse failure. Callers treat any `Err` as fail-closed: a block that
/// carries a check's textual trigger but that the real parser cannot model is
/// reported as a violation, never skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The parsed shape of a fenced block. `Map` preserves author order so a
/// scanner can report the first occurrence of a key.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Map(Vec<Entry>),
    Seq(Vec<Node>),
    /// A concrete, fully-decoded scalar literal in value position.
    Scalar(String),
    Null,
    /// The `...` abbreviation marker: "more fields, deliberately not shown".
    Elision,
    /// An authored `<placeholder>` such as `<name>` or `<ResourceType>`.
    Placeholder,
    /// A non-literal scalar value, such as Nix string interpolation or a bare
    /// variable reference, that is present but not concrete enough for a
    /// value-position check to inspect.
    Opaque,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: String,
    pub val: Node,
    /// Block-relative 0-based line index of the key, best-effort.
    pub line: usize,
}

/// A direct child of a mapping, by normalized key.
pub fn direct_child<'a>(map: &'a [Entry], key: &str) -> Option<&'a Entry> {
    map.iter().find(|e| e.key == key)
}

/// Collect every mapping in the document tree (the node and every descendant).
pub fn collect_maps<'a>(node: &'a Node, out: &mut Vec<&'a [Entry]>) {
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

/// A repository-relative display string for a path, so a lint diagnostic or a
/// panic message never prints the checkout root or a username-bearing absolute
/// path into a CI log. Falls back to the final path component - never the
/// absolute path - when the path is not under the repository root.
pub fn rel_display(path: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(d2b_contract_tests::repo_root()) {
        return rel.to_string_lossy().into_owned();
    }
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "<redacted>".to_string())
}

// ---------------------------------------------------------------------------
// Fenced-code-block model.
// ---------------------------------------------------------------------------

pub struct Block<'a> {
    pub lang: String,
    pub body_start: usize,
    pub lines: Vec<&'a str>,
}

pub fn fenced_blocks(content: &str) -> Vec<Block<'_>> {
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

/// Whether a line carries `key` (optionally quoted) followed by `:` or `=`,
/// anywhere on the line. Used only to detect that a block clearly intends a
/// given authoring context so a scanner can fail closed if the parser cannot
/// model it.
pub fn mentions_key(lines: &[&str], key: &str) -> bool {
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

/// Best-effort 0-based block-relative line of the first author line that
/// introduces `key` (as `key:`, `"key":`, `key =`, or a dotted `key.`). Used
/// only to point a violation at a real source line; correctness of the finding
/// never depends on it.
pub fn key_line(lines: &[&str], key: &str) -> Option<usize> {
    lines.iter().position(|l| {
        let t = l.trim_start();
        let t = t.strip_prefix('"').unwrap_or(t);
        if let Some(after) = t.strip_prefix(key) {
            let after = after.trim_start_matches('"');
            return after.starts_with(':')
                || after.trim_start().starts_with('=')
                || after.starts_with('.');
        }
        false
    })
}

// ---------------------------------------------------------------------------
// Sentinels for documented schematic conventions.
// ---------------------------------------------------------------------------

const PH_SENTINEL: &str = "__d2b_ph__";
const ELIDE_SENTINEL: &str = "__d2b_elided__";
const OPAQUE_SENTINEL: &str = "__d2b_opaque__";

/// Synthetic entry key under which a `d2b-lint: <token>` comment is surfaced
/// into the parsed model. A lint that pins an intentional negative example to a
/// specific resource reads the marker as a direct child of the exact map or
/// list element that lexically contains the comment, so the marker binds to one
/// parsed resource rather than a whole fenced block. Only Nix carries comments
/// through the parser; JSON has no comments and a YAML parser discards them, so
/// a marker only ever surfaces from a Nix fence.
pub const LINT_MARKER_KEY: &str = "__d2b_lint_marker__";
const STRUCTURAL_CHILD_KEY: &str = "__d2b_structural_child__";

/// The `<token>` of a `d2b-lint: <token>` comment, or `None` for any other
/// comment. Strips a `#` line comment or a `/* ... */` block comment and the
/// `d2b-lint:` prefix, so only the deliberate lint marker surfaces.
fn lint_marker_from_comment(text: &str) -> Option<String> {
    let mut t = text.trim();
    if let Some(rest) = t.strip_prefix("/*") {
        t = rest.strip_suffix("*/").unwrap_or(rest);
    }
    let t = t.trim_start_matches(['#', '/']).trim();
    let rest = t.strip_prefix("d2b-lint:")?;
    Some(rest.trim().to_string())
}

fn scalar_to_node(s: String) -> Node {
    if s == "..." {
        Node::Elision
    } else if s == OPAQUE_SENTINEL {
        Node::Opaque
    } else if s == PH_SENTINEL || (s.starts_with('<') && s.ends_with('>') && s.len() >= 2) {
        Node::Placeholder
    } else {
        Node::Scalar(s)
    }
}

// ---------------------------------------------------------------------------
// Dispatch.
// ---------------------------------------------------------------------------

pub fn parse_block_docs(lang: &str, lines: &[&str]) -> Result<Vec<Node>, ParseError> {
    match lang {
        "yaml" | "yml" => parse_yaml(lines),
        "json" => parse_json(lines).map(|n| vec![n]),
        "nix" => parse_nix(lines).map(|n| vec![n]),
        "" => {
            // An untagged fence is genuinely ambiguous. Try each real parser and
            // accept the first that models the block cleanly; a `{`/`[` opener
            // hints JSON first, otherwise YAML first. Fail closed only when NONE
            // of the three parsers accept it, so an untagged block is never
            // silently skipped just because the first guess was wrong.
            let joined = lines.join("\n");
            let order: [&str; 3] = match joined.trim_start().chars().next() {
                Some('{') | Some('[') => ["json", "nix", "yaml"],
                _ => ["yaml", "nix", "json"],
            };
            let mut last: Option<ParseError> = None;
            for lang in order {
                let attempt = match lang {
                    "yaml" => parse_yaml(lines),
                    "nix" => parse_nix(lines).map(|n| vec![n]),
                    _ => parse_json(lines).map(|n| vec![n]),
                };
                match attempt {
                    Ok(n) => return Ok(n),
                    Err(e) => last = Some(e),
                }
            }
            Err(last.unwrap_or_else(|| ParseError("untagged block did not parse".to_string())))
        }
        _ => Ok(Vec::new()),
    }
}

// ---------------------------------------------------------------------------
// YAML: serde_yaml_ng with a duplicate-key-tolerant generic visitor.
//
// The generic visitor keeps every entry a mapping declares (serde_yaml_ng's
// own `Value` rejects a duplicate key, which would fail-close on a pre-existing
// schema collision unrelated to any contract this module enforces). Anchors and
// aliases are resolved by serde_yaml_ng before the visitor runs; `\uXXXX`
// escapes in a double-quoted key or value are decoded; a `<<` merge key is
// folded during conversion to `Node`.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum YNode {
    Map(Vec<(YNode, YNode)>),
    Seq(Vec<YNode>),
    Str(String),
    Bool(bool),
    Num(String),
    Null,
}

impl<'de> Deserialize<'de> for YNode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = YNode;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any YAML node")
            }
            fn visit_bool<E>(self, v: bool) -> Result<YNode, E> {
                Ok(YNode::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<YNode, E> {
                Ok(YNode::Num(v.to_string()))
            }
            fn visit_u64<E>(self, v: u64) -> Result<YNode, E> {
                Ok(YNode::Num(v.to_string()))
            }
            fn visit_i128<E>(self, v: i128) -> Result<YNode, E> {
                Ok(YNode::Num(v.to_string()))
            }
            fn visit_u128<E>(self, v: u128) -> Result<YNode, E> {
                Ok(YNode::Num(v.to_string()))
            }
            fn visit_f64<E>(self, v: f64) -> Result<YNode, E> {
                Ok(YNode::Num(v.to_string()))
            }
            fn visit_str<E>(self, v: &str) -> Result<YNode, E> {
                Ok(YNode::Str(v.to_string()))
            }
            fn visit_string<E>(self, v: String) -> Result<YNode, E> {
                Ok(YNode::Str(v))
            }
            fn visit_unit<E>(self) -> Result<YNode, E> {
                Ok(YNode::Null)
            }
            fn visit_none<E>(self) -> Result<YNode, E> {
                Ok(YNode::Null)
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<YNode, D::Error> {
                YNode::deserialize(d)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<YNode, A::Error> {
                let mut items = Vec::new();
                while let Some(v) = a.next_element::<YNode>()? {
                    items.push(v);
                }
                Ok(YNode::Seq(items))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<YNode, A::Error> {
                let mut entries = Vec::new();
                while let Some((k, v)) = a.next_entry::<YNode, YNode>()? {
                    entries.push((k, v));
                }
                Ok(YNode::Map(entries))
            }
        }
        d.deserialize_any(V)
    }
}

fn ynode_key(k: &YNode) -> String {
    match k {
        YNode::Str(s) => s.clone(),
        YNode::Bool(b) => b.to_string(),
        YNode::Num(n) => n.clone(),
        YNode::Null => "null".to_string(),
        YNode::Map(_) | YNode::Seq(_) => "__d2b_complex_key__".to_string(),
    }
}

fn ynode_to_node(y: &YNode) -> Node {
    match y {
        YNode::Str(s) => scalar_to_node(s.clone()),
        YNode::Bool(b) => Node::Scalar(b.to_string()),
        YNode::Num(n) => Node::Scalar(n.clone()),
        YNode::Null => Node::Null,
        YNode::Seq(items) => Node::Seq(items.iter().map(ynode_to_node).collect()),
        YNode::Map(pairs) => Node::Map(ynode_entries(pairs)),
    }
}

fn ynode_entries(pairs: &[(YNode, YNode)]) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut merges: Vec<&YNode> = Vec::new();
    for (k, v) in pairs {
        let key = ynode_key(k);
        if key == "<<" {
            merges.push(v);
            continue;
        }
        if key == ELIDE_SENTINEL {
            entries.push(Entry {
                key: "...".to_string(),
                val: Node::Elision,
                line: 0,
            });
            continue;
        }
        entries.push(Entry {
            key,
            val: ynode_to_node(v),
            line: 0,
        });
    }
    // Fold YAML merge keys: explicit keys win over merged ones.
    for m in merges {
        let mut maps: Vec<&Vec<(YNode, YNode)>> = Vec::new();
        match m {
            YNode::Map(p) => maps.push(p),
            YNode::Seq(items) => {
                for it in items {
                    if let YNode::Map(p) = it {
                        maps.push(p);
                    }
                }
            }
            _ => {}
        }
        for p in maps {
            for e in ynode_entries(p) {
                if !entries.iter().any(|x| x.key == e.key) {
                    entries.push(e);
                }
            }
        }
    }
    entries
}

fn normalize_yaml(lines: &[&str]) -> String {
    let mut out = Vec::with_capacity(lines.len());
    for l in lines {
        let t = l.trim();
        let ind = l.len() - l.trim_start_matches(' ').len();
        // A bare `...` line is a mapping-level elision marker.
        if t == "..." {
            out.push(format!("{}{}: true", " ".repeat(ind), ELIDE_SENTINEL));
            continue;
        }
        let mut line = (*l).to_string();
        // A schematic `key: A | B | C` union (a documented "one of" notation)
        // collapses to an opaque sentinel. A quoted alternative
        // (`"True" | "False"`) is otherwise a hard YAML syntax error, and a bare
        // union is a meaningless multi-word string; either way it is not a
        // concrete literal a value-position check can judge.
        if let Some(collapsed) = collapse_yaml_union(&line) {
            line = collapsed;
        } else if !line.contains('"') && !line.contains('\'') {
            // Flow-collection elision embedded in a value, e.g. `spec: { ... }`.
            line = collapse_flow_elision(&line);
        }
        out.push(line);
    }
    out.join("\n")
}

/// Replace quoted spans in `s` with a single space so a `|` inside a quoted
/// scalar is never read as a union separator.
fn strip_quoted_spans(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '"' || c == '\'' {
            for n in chars.by_ref() {
                if n == c {
                    break;
                }
            }
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// If `line` is a `key: value` line whose value (outside quotes and any trailing
/// comment) is a ` | `-separated union of alternatives, rewrite the value to the
/// opaque sentinel and return it; otherwise return `None`. The head (indent,
/// list marker, key, and `: ` separator) is preserved verbatim.
fn collapse_yaml_union(line: &str) -> Option<String> {
    let sep = line.find(": ")?;
    let (head, value) = line.split_at(sep + 2);
    let stripped = strip_quoted_spans(value);
    let code = match stripped.find('#') {
        Some(i) => &stripped[..i],
        None => stripped.as_str(),
    };
    if code.contains(" | ") {
        Some(format!("{head}{OPAQUE_SENTINEL}"))
    } else {
        None
    }
}

fn parse_yaml(lines: &[&str]) -> Result<Vec<Node>, ParseError> {
    let text = normalize_yaml(lines);
    let mut out = Vec::new();
    for de in serde_yaml_ng::Deserializer::from_str(&text) {
        let y = YNode::deserialize(de).map_err(|e| ParseError(e.to_string()))?;
        out.push(ynode_to_node(&y));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Flow / prose elision collapse (shared by JSON and Nix normalizers, and by
// the YAML flow-value normalizer). A single-line `{ ...prose... }` or
// `[ ...prose... ]` is a documented "spec fields omitted" abbreviation; it
// collapses to an empty collection so the real parser accepts it.
// ---------------------------------------------------------------------------

fn collapse_flow_elision(s: &str) -> String {
    fn collapse(input: &str, open: char, close: char) -> String {
        let bytes: Vec<char> = input.chars().collect();
        let mut out = String::with_capacity(input.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == open {
                // Find the first close/open/newline after this open; only
                // collapse a `{ ... }` with no inner brace and a `...` inside.
                if let Some(j) = (i + 1..bytes.len())
                    .find(|&j| bytes[j] == close || bytes[j] == open || bytes[j] == '\n')
                    && bytes[j] == close
                {
                    let inner: String = bytes[i + 1..j].iter().collect();
                    if inner.contains("...") {
                        out.push(open);
                        out.push(close);
                        i = j + 1;
                        continue;
                    }
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        out
    }
    let mut cur = s.to_string();
    for _ in 0..8 {
        let next = collapse(&cur, '{', '}');
        let next = collapse(&next, '[', ']');
        if next == cur {
            break;
        }
        cur = next;
    }
    cur
}

// ---------------------------------------------------------------------------
// JSON: serde_json, with a string-aware normalizer that strips `//` and
// `/* */` comments and rewrites `<placeholder>` tokens and flow elision to
// sentinels the parser accepts. serde_json decodes `\uXXXX` escapes natively,
// so an escaped key such as `"ty\u0070e"` resolves to `type`.
// ---------------------------------------------------------------------------

fn normalize_json_core(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut in_str = false;
    while i < b.len() {
        let c = b[i] as char;
        if in_str {
            if c == '\\' && i + 1 < b.len() {
                out.push(c);
                out.push(b[i + 1] as char);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
                out.push(c);
                i += 1;
                continue;
            }
            if c == '<'
                && let Some(rel) = text[i..].find('>')
            {
                let end = i + rel;
                if !text[i..end].contains('\n') && end - i <= 64 {
                    out.push_str(PH_SENTINEL);
                    i = end + 1;
                    continue;
                }
            }
            out.push(c);
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '/' && i + 1 < b.len() && b[i + 1] as char == '/' {
            while i < b.len() && b[i] as char != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < b.len() && b[i + 1] as char == '*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] as char == '*' && b[i + 1] as char == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == '<'
            && let Some(rel) = text[i..].find('>')
        {
            let end = i + rel;
            if !text[i..end].contains('\n') && end - i <= 64 {
                out.push('"');
                out.push_str(PH_SENTINEL);
                out.push('"');
                i = end + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

fn normalize_json(lines: &[&str]) -> String {
    let core = normalize_json_core(&lines.join("\n"));
    let collapsed = collapse_flow_elision(&core);
    let mut out = Vec::new();
    for l in collapsed.lines() {
        if l.trim() == "..." {
            let ind = l.len() - l.trim_start_matches(' ').len();
            out.push(format!("{}\"{}\": true", " ".repeat(ind), ELIDE_SENTINEL));
        } else {
            out.push(l.to_string());
        }
    }
    out.join("\n")
}

// Deserialize through a map visitor instead of serde_json::Value so duplicate
// keys remain distinct entries. The envelope scanner can then apply the same
// discriminator-cardinality rule to JSON, YAML, and Nix instead of inheriting
// serde_json::Map's last-wins collapse.
fn parse_json(lines: &[&str]) -> Result<Node, ParseError> {
    let text = normalize_json(lines);
    if text.trim().is_empty() {
        return Ok(Node::Null);
    }
    let mut deserializer = serde_json::Deserializer::from_str(&text);
    let node = JsonNode::deserialize(&mut deserializer)
        .map_err(|error| ParseError(error.to_string()))?
        .0;
    deserializer
        .end()
        .map_err(|error| ParseError(error.to_string()))?;
    Ok(node)
}

struct JsonNode(Node);

impl<'de> Deserialize<'de> for JsonNode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct JsonNodeVisitor;

        impl<'de> Visitor<'de> for JsonNodeVisitor {
            type Value = JsonNode;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON value")
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
                Ok(JsonNode(Node::Scalar(value.to_string())))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
                Ok(JsonNode(Node::Scalar(value.to_string())))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
                Ok(JsonNode(Node::Scalar(value.to_string())))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
                Ok(JsonNode(Node::Scalar(value.to_string())))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(JsonNode(scalar_to_node(value.to_string())))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(JsonNode(scalar_to_node(value)))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(JsonNode(Node::Null))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(JsonNode(Node::Null))
            }

            fn visit_some<D: Deserializer<'de>>(
                self,
                deserializer: D,
            ) -> Result<Self::Value, D::Error> {
                JsonNode::deserialize(deserializer)
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = sequence.next_element::<JsonNode>()? {
                    items.push(item.0);
                }
                Ok(JsonNode(Node::Seq(items)))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut mapping: A) -> Result<Self::Value, A::Error> {
                let mut entries: Vec<Entry> = Vec::new();
                while let Some(key) = mapping.next_key::<String>()? {
                    let key = if key == ELIDE_SENTINEL || key == "..." {
                        "...".to_string()
                    } else {
                        key
                    };
                    let value = mapping.next_value::<JsonNode>()?.0;
                    let value = if key == "..." { Node::Elision } else { value };
                    entries.push(Entry {
                        key,
                        val: value,
                        line: 0,
                    });
                }
                Ok(JsonNode(Node::Map(entries)))
            }
        }

        deserializer.deserialize_any(JsonNodeVisitor)
    }
}

// ---------------------------------------------------------------------------
// Nix: rnix. Every node SyntaxKind has an explicit disposition:
//
// * NODE_ATTR_SET covers both `{ ... }` and `rec { ... }`.
// * NODE_ROOT and NODE_PAREN require and expose their sole expression.
// * NODE_WITH, NODE_LET_IN, NODE_IF_ELSE, NODE_ASSERT, NODE_LAMBDA, NODE_LIST,
//   NODE_APPLY, NODE_BIN_OP, NODE_SELECT, NODE_HAS_ATTR, and NODE_UNARY_OP expose
//   every child expression. Conditions, predicates, lambda parameters/defaults,
//   namespaces, bindings, operands, sources, defaults, and result bodies are all
//   visited rather than selected by position.
// * NODE_ATTRPATH_VALUE, NODE_ATTRPATH, NODE_DYNAMIC, NODE_INHERIT,
//   NODE_INHERIT_FROM, NODE_INTERPOL, NODE_PATTERN, NODE_PAT_ENTRY, and
//   NODE_PAT_BIND expose every child. This includes dynamic path expressions,
//   inherit sources, string/path interpolation, and lambda parameter defaults.
// * NODE_LEGACY_LET uses the same entry walker as NODE_ATTR_SET, including every
//   value and inherit source.
// * Static `inherit` names become opaque entries, preserving the attribute-set
//   shape; dynamic/interpolated attribute names fail closed because their
//   semantic key cannot be known without evaluation.
// * NODE_STRING and NODE_PATH_* are scalar/opaque when childless and expose all
//   interpolations otherwise. NODE_IDENT, NODE_IDENT_PARAM, NODE_LITERAL, and
//   NODE_CUR_POS are childless leaves; an unexpected future child is still
//   visited rather than dropped.
// * NODE_ERROR always fails. Internal node kinds in expression position are
//   traversed as above, while semantically unresolvable dynamic attribute names
//   fail in the attribute-set entry walker.
//
// Any future or misplaced structural SyntaxKind also fails closed through the
// wildcard arm. Angle-bracket placeholders (`<name>`) would otherwise tokenize
// as Nix lookup-path syntax, so they are rewritten to a sentinel identifier
// before parsing, and single-line prose elision (`{ ...fields... }`) collapses
// to an empty set.
// ---------------------------------------------------------------------------

fn normalize_nix(text: &str) -> String {
    let angle = rewrite_angle_placeholders(text);
    collapse_flow_elision(&angle)
}

/// Replace `<ident-ish>` angle-bracket placeholders with a sentinel identifier.
/// Conservative: only `<` immediately followed by an ASCII letter with a `>` on
/// the same line and an identifier-shaped interior.
fn rewrite_angle_placeholders(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<'
            && i + 1 < chars.len()
            && chars[i + 1].is_ascii_alphabetic()
            && let Some(rel) = (i + 1..chars.len()).find(|&j| chars[j] == '>' || chars[j] == '\n')
            && chars[rel] == '>'
        {
            let inner: String = chars[i + 1..rel].iter().collect();
            if inner
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                out.push_str(PH_SENTINEL);
                i = rel + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn parse_nix(lines: &[&str]) -> Result<Node, ParseError> {
    let norm = normalize_nix(&lines.join("\n"));
    if norm.trim().is_empty() {
        return Ok(Node::Null);
    }
    let attempts = [norm.clone(), format!("{{\n{norm}\n}}")];
    let mut last_err = String::new();
    for attempt in &attempts {
        let parse = rnix::Root::parse(attempt);
        let errs = parse.errors();
        if errs.is_empty() {
            return nix_node_to_node(&parse.syntax());
        }
        last_err = errs
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ");
    }
    Err(ParseError(last_err))
}

fn nix_children(node: &SyntaxNode) -> Result<Vec<Node>, ParseError> {
    node.children()
        .map(|child| nix_node_to_node(&child))
        .collect()
}

fn nix_children_seq(node: &SyntaxNode) -> Result<Node, ParseError> {
    nix_children(node).map(Node::Seq)
}

fn nix_exact_children_seq(node: &SyntaxNode, expected: usize) -> Result<Node, ParseError> {
    let children = nix_children(node)?;
    if children.len() != expected {
        return Err(ParseError(format!(
            "{:?} has {} child expressions, expected {expected}; refusing to skip a structural \
             Nix child",
            node.kind(),
            children.len()
        )));
    }
    Ok(Node::Seq(children))
}

fn nix_single_child(node: &SyntaxNode) -> Result<Node, ParseError> {
    let mut children = nix_children(node)?;
    if children.len() != 1 {
        return Err(ParseError(format!(
            "{:?} has {} child expressions, expected exactly one; refusing to skip a structural \
             Nix child",
            node.kind(),
            children.len()
        )));
    }
    Ok(children
        .pop()
        .expect("length checked before extracting sole child"))
}

fn nix_leaf_or_children(node: &SyntaxNode, leaf: Node) -> Result<Node, ParseError> {
    let children = nix_children(node)?;
    if children.is_empty() {
        Ok(leaf)
    } else {
        Ok(Node::Seq(children))
    }
}

fn nix_node_to_node(node: &SyntaxNode) -> Result<Node, ParseError> {
    match node.kind() {
        SyntaxKind::NODE_ATTR_SET => Ok(Node::Map(nix_attrset_entries(node)?)),
        SyntaxKind::NODE_LIST => nix_children_seq(node),
        SyntaxKind::NODE_STRING => nix_string_node(node),
        SyntaxKind::NODE_LITERAL => nix_leaf_or_children(
            node,
            Node::Scalar(node.text().to_string().trim().to_string()),
        ),
        SyntaxKind::NODE_IDENT => nix_leaf_or_children(node, nix_ident_node(node)),
        SyntaxKind::NODE_IDENT_PARAM => nix_leaf_or_children(node, Node::Opaque),
        SyntaxKind::NODE_WITH => nix_exact_children_seq(node, 2),
        SyntaxKind::NODE_LET_IN => nix_children_seq(node),
        SyntaxKind::NODE_PAREN | SyntaxKind::NODE_ROOT => nix_single_child(node),
        SyntaxKind::NODE_IF_ELSE => nix_exact_children_seq(node, 3),
        SyntaxKind::NODE_APPLY | SyntaxKind::NODE_BIN_OP => nix_children_seq(node),
        SyntaxKind::NODE_ASSERT | SyntaxKind::NODE_LAMBDA => nix_exact_children_seq(node, 2),
        SyntaxKind::NODE_LEGACY_LET => Ok(Node::Map(nix_attrset_entries(node)?)),
        SyntaxKind::NODE_SELECT
        | SyntaxKind::NODE_HAS_ATTR
        | SyntaxKind::NODE_UNARY_OP
        | SyntaxKind::NODE_ATTRPATH
        | SyntaxKind::NODE_ATTRPATH_VALUE
        | SyntaxKind::NODE_DYNAMIC
        | SyntaxKind::NODE_INHERIT
        | SyntaxKind::NODE_INHERIT_FROM
        | SyntaxKind::NODE_INTERPOL
        | SyntaxKind::NODE_PAT_BIND
        | SyntaxKind::NODE_PAT_ENTRY
        | SyntaxKind::NODE_PATTERN => nix_children_seq(node),
        SyntaxKind::NODE_CUR_POS => nix_leaf_or_children(node, Node::Opaque),
        SyntaxKind::NODE_PATH_ABS
        | SyntaxKind::NODE_PATH_HOME
        | SyntaxKind::NODE_PATH_REL
        | SyntaxKind::NODE_PATH_SEARCH => nix_leaf_or_children(node, Node::Opaque),
        SyntaxKind::NODE_ERROR => Err(ParseError(
            "rnix produced an error node in a parsed Nix expression".to_string(),
        )),
        _ => Err(ParseError(format!(
            "unsupported Nix syntax {:?}; refusing to skip a possible structural wrapper",
            node.kind()
        ))),
    }
}

fn nix_ident_node(node: &SyntaxNode) -> Node {
    let t = node.text().to_string();
    let t = t.trim();
    match t {
        "null" => Node::Null,
        "true" | "false" => Node::Scalar(t.to_string()),
        PH_SENTINEL => Node::Placeholder,
        _ => Node::Opaque,
    }
}

fn nix_string_node(node: &SyntaxNode) -> Result<Node, ParseError> {
    let children = nix_children(node)?;
    if !children.is_empty() {
        return Ok(Node::Seq(children));
    }
    Ok(scalar_to_node(nix_static_string(node)?))
}

fn nix_static_string(node: &SyntaxNode) -> Result<String, ParseError> {
    if node.children().count() != 0 {
        return Err(ParseError(
            "interpolated Nix string is not a static literal".to_string(),
        ));
    }
    let mut content = String::new();
    for tok in node.children_with_tokens() {
        if let Some(t) = tok.as_token()
            && t.kind() == SyntaxKind::TOKEN_STRING_CONTENT
        {
            content.push_str(t.text());
        }
    }
    Ok(unescape_nix(&content))
}

fn unescape_nix(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('$') => out.push('$'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Walk a `NODE_ATTR_SET`'s `NODE_ATTRPATH_VALUE` children, expanding dotted
/// attrpaths (`metadata.name = x`) into nested maps and merging paths that
/// share a prefix.
fn nix_attrset_entries(node: &SyntaxNode) -> Result<Vec<Entry>, ParseError> {
    let mut entries: Vec<Entry> = Vec::new();
    for child in node.children() {
        match child.kind() {
            SyntaxKind::NODE_ATTRPATH_VALUE => {
                let mut path_nodes = child
                    .children()
                    .filter(|candidate| candidate.kind() == SyntaxKind::NODE_ATTRPATH)
                    .collect::<Vec<_>>();
                if path_nodes.len() != 1 {
                    return Err(ParseError(
                        "Nix attribute value does not have exactly one attribute path".to_string(),
                    ));
                }
                let path_node = path_nodes
                    .pop()
                    .expect("length checked before extracting sole attribute path");
                let segments = nix_attrpath_segments(&path_node)?;
                if segments.is_empty() {
                    return Err(ParseError("Nix attribute path is empty".to_string()));
                }
                let value = nix_attrpath_value(&child)?;
                insert_path(&mut entries, &segments, value);
            }
            SyntaxKind::NODE_INHERIT => entries.extend(nix_inherit_entries(&child)?),
            kind => {
                return Err(ParseError(format!(
                    "unexpected {kind:?} inside a Nix attribute set; refusing to skip an entry"
                )));
            }
        }
    }
    // Surface any `d2b-lint: <token>` comment that is a direct child of this
    // attrset as a synthetic entry, so a lint can bind an intentional-negative
    // marker to the exact resource that lexically contains it rather than to
    // the whole fenced block.
    for tok in node.children_with_tokens() {
        if let Some(t) = tok.as_token()
            && t.kind() == SyntaxKind::TOKEN_COMMENT
            && let Some(marker) = lint_marker_from_comment(t.text())
        {
            entries.push(Entry {
                key: LINT_MARKER_KEY.to_string(),
                val: Node::Scalar(marker),
                line: 0,
            });
        }
    }
    Ok(entries)
}

fn nix_inherit_entries(node: &SyntaxNode) -> Result<Vec<Entry>, ParseError> {
    let mut entries = Vec::new();
    for attr in node.children() {
        match attr.kind() {
            SyntaxKind::NODE_INHERIT_FROM => entries.push(Entry {
                key: STRUCTURAL_CHILD_KEY.to_string(),
                val: nix_node_to_node(&attr)?,
                line: 0,
            }),
            SyntaxKind::NODE_IDENT => entries.push(Entry {
                key: attr.text().to_string().trim().to_string(),
                val: Node::Opaque,
                line: 0,
            }),
            SyntaxKind::NODE_STRING => {
                if attr
                    .children()
                    .any(|child| child.kind() == SyntaxKind::NODE_INTERPOL)
                {
                    return Err(ParseError(
                        "interpolated inherited attribute name cannot be modelled without \
                         evaluation"
                            .to_string(),
                    ));
                }
                let key = nix_static_string(&attr)?;
                entries.push(Entry {
                    key,
                    val: Node::Opaque,
                    line: 0,
                });
            }
            SyntaxKind::NODE_DYNAMIC => {
                return Err(ParseError(
                    "dynamic inherited attribute name cannot be modelled without evaluation"
                        .to_string(),
                ));
            }
            kind => {
                return Err(ParseError(format!(
                    "unexpected {kind:?} in a Nix inherit expression"
                )));
            }
        }
    }
    Ok(entries)
}

fn nix_attrpath_value(node: &SyntaxNode) -> Result<Node, ParseError> {
    let values = node
        .children()
        .filter(|child| child.kind() != SyntaxKind::NODE_ATTRPATH)
        .map(|value| nix_node_to_node(&value))
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() != 1 {
        return Err(ParseError(format!(
            "Nix attribute value has {} value expressions, expected exactly one",
            values.len()
        )));
    }
    values
        .into_iter()
        .next()
        .ok_or_else(|| ParseError("Nix attribute value disappeared after validation".to_string()))
}

fn nix_attrpath_segments(path: &SyntaxNode) -> Result<Vec<String>, ParseError> {
    let mut segs = Vec::new();
    for seg in path.children() {
        match seg.kind() {
            SyntaxKind::NODE_IDENT => segs.push(seg.text().to_string().trim().to_string()),
            SyntaxKind::NODE_STRING => {
                if seg
                    .children()
                    .any(|child| child.kind() == SyntaxKind::NODE_INTERPOL)
                {
                    return Err(ParseError(
                        "interpolated Nix attribute name cannot be modelled without evaluation"
                            .to_string(),
                    ));
                }
                segs.push(nix_static_string(&seg)?);
            }
            SyntaxKind::NODE_DYNAMIC => {
                return Err(ParseError(
                    "dynamic Nix attribute name cannot be modelled without evaluation".to_string(),
                ));
            }
            kind => {
                return Err(ParseError(format!(
                    "unexpected {kind:?} in a Nix attribute path"
                )));
            }
        }
    }
    Ok(segs)
}

fn insert_path(entries: &mut Vec<Entry>, segments: &[String], value: Node) {
    let key = &segments[0];
    if segments.len() == 1 {
        entries.push(Entry {
            key: key.clone(),
            val: value,
            line: 0,
        });
        return;
    }
    if let Some(existing) = entries.iter_mut().find(|e| &e.key == key)
        && let Node::Map(inner) = &mut existing.val
    {
        insert_path(inner, &segments[1..], value);
        return;
    }
    let mut inner = Vec::new();
    insert_path(&mut inner, &segments[1..], value);
    entries.push(Entry {
        key: key.clone(),
        val: Node::Map(inner),
        line: 0,
    });
}
