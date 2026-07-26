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
//! The parsing itself is delegated to real parsers behind the shared `common`
//! module: `serde_json` for JSON, `serde_yaml_ng` for YAML, and `rnix` for Nix.
//! Every parse returns a `Result` with an explicit error channel, and both
//! scanners treat a parse error on a block that intends an envelope as a
//! violation (fail closed), never as "nothing to check". Comments, documented
//! `...` elision, and authored `<placeholder>` tokens are recognized as
//! first-class conventions and normalized into distinct `Node` variants rather
//! than skipped, so a commented-out key can never satisfy a rule and an elision
//! is honoured only where the contract allows it.
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

mod common;

use common::{
    Entry, LINT_MARKER_KEY, Node, collect_maps, direct_child, fenced_blocks, key_line,
    mentions_key, parse_block_docs, rel_display,
};

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
// exactly one file, one marker, and one resource: it applies only in
// `docs/specs/ADR-046-nix-configuration.md`, only when the exact comment
// `d2b-lint: expect-d116-eval-error` occurs exactly once in the file, and then
// only to the single parsed resource that lexically contains that marker. The
// marker is read from the parsed document (a synthetic child surfaced by the Nix
// parser), so a second unmarked resource in the same fence is still reported.
// The same marker in any other file, or a duplicated marker, does not suppress
// anything - it fails closed.
// ---------------------------------------------------------------------------

const D116_EXEMPT_FILE: &str = "docs/specs/ADR-046-nix-configuration.md";
const D116_EXEMPT_MARKER: &str = "d2b-lint: expect-d116-eval-error";
const D116_EXEMPT_MARKER_TOKEN: &str = "expect-d116-eval-error";

/// Whether `line` is exactly the pinned D116 negative-example marker comment.
/// Only a comment line (`#` or `//`) carrying exactly the marker text qualifies,
/// so a stray mention in prose or a string value cannot suppress a violation.
/// Used only to count markers across the file for the exactly-once file guard;
/// the per-resource binding reads the parsed marker instead.
fn is_d116_marker_line(line: &str) -> bool {
    let t = line.trim();
    let t = t.trim_start_matches(['#', '/']).trim();
    t == D116_EXEMPT_MARKER
}

/// Whether this exact resource map is the pinned D116 negative example: it
/// carries the parsed `d2b-lint: expect-d116-eval-error` marker as a direct
/// child or one level down under its `spec`. Binding to the parsed marker rather
/// than to any marker line in the fence keeps the exemption scoped to one
/// resource, so an unmarked violating sibling in the same fence is still flagged.
fn resource_carries_d116_marker(map: &[Entry]) -> bool {
    resource_entry(map, LINT_MARKER_KEY)
        .map(|e| matches!(&e.val, Node::Scalar(s) if s == D116_EXEMPT_MARKER_TOKEN))
        .unwrap_or(false)
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
/// non-null scalar. A `<placeholder>` ref or a Nix variable/interpolation
/// (`Opaque`) is a present, deliberately-abstract ref and satisfies the
/// invariant; only a missing key, `null`, or an empty string does not.
fn ref_is_set(node: Option<&Node>) -> bool {
    match node {
        Some(Node::Scalar(s)) => !s.is_empty() && s != "null",
        Some(Node::Placeholder) | Some(Node::Opaque) => true,
        _ => false,
    }
}

/// Whether a block clearly intends a Host/Guest execution-policy authoring
/// context that admits the `user` domain. Used only to decide whether an
/// unparseable block must fail closed: a block the real parser cannot model,
/// but that plainly names a Host/Guest `type`, an `allowedDomains` list, and
/// the `user` domain, is a parser gap the lint reports rather than skips.
fn intends_user_domain_host_guest(lines: &[&str]) -> bool {
    let names_type = (mentions_key(lines, "type") || mentions_key(lines, "resourceType"))
        && lines
            .iter()
            .any(|l| l.contains("Host") || l.contains("Guest"));
    let names_domains = mentions_key(lines, "allowedDomains");
    let names_user = lines.iter().any(|l| l.contains("user"));
    names_type && names_domains && names_user
}

fn scan_d116(file: &str, content: &str) -> Vec<Violation> {
    let marker_count = content.lines().filter(|l| is_d116_marker_line(l)).count();
    let exemption_active = file == D116_EXEMPT_FILE && marker_count == 1;
    let mut out = Vec::new();
    for block in fenced_blocks(content) {
        if !matches!(block.lang.as_str(), "yaml" | "yml" | "json" | "nix" | "") {
            continue;
        }
        let docs = match parse_block_docs(&block.lang, &block.lines) {
            Ok(docs) => docs,
            Err(_) => {
                // Fail closed: a block the real parser cannot model but that
                // plainly declares a user-domain Host/Guest is a parser gap, not
                // a pass. The one exception is the pinned negative example: when
                // the exemption is active and this unparseable block carries the
                // marker line, it is the deliberate teaching block and is not
                // flagged. A per-resource decision is impossible without a parse
                // tree, so this fallback is intentionally coarse.
                if intends_user_domain_host_guest(&block.lines)
                    && !(exemption_active && block.lines.iter().any(|l| is_d116_marker_line(l)))
                {
                    out.push(Violation {
                        file: file.to_string(),
                        line: block.body_start + 1,
                        text: "block declares a user-domain Host/Guest the structural parser could not model; a lint must fail closed on an unparseable execution-policy block, not skip it (D116)"
                            .to_string(),
                    });
                }
                continue;
            }
        };
        for doc in &docs {
            let mut maps = Vec::new();
            collect_maps(doc, &mut maps);
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
                // Per-resource exemption: suppress only the single parsed
                // resource that carries the pinned marker, never a sibling.
                if exemption_active && resource_carries_d116_marker(m) {
                    continue;
                }
                let line =
                    block.body_start + key_line(&block.lines, "allowedDomains").unwrap_or(0) + 1;
                out.push(Violation {
                    file: file.to_string(),
                    line,
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
// A live envelope is any fenced YAML/JSON/Nix document that carries an
// `apiVersion` key and a `type` scalar. When it also shows a `status`, that
// status subtree must carry both `update` and `resource` as direct children. A
// subtree that carries `credential`, `device`, or any other
// ResourceType-specific key in place of `resource` is a violation: the Layer-2
// key is frozen as `resource`.
//
// Elision is honoured narrowly: only a bare `...` marker key that is a direct
// child of `status`, or an inline `status: ...`, abbreviates the status body. A
// `...` that is the VALUE of some other key (`conditions: ...`) does not, nor
// does a `...` nested deeper. An inline `status: {}` or `status: null` on a live
// envelope is incomplete rather than abbreviated.
//
// A bundle envelope (an `apiVersion` document whose type field is `resourceType`
// rather than `type`) is the compiler-emitted contract that deliberately carries
// `status: null`; it is recognized as distinct and is not required to carry a
// status base.
//
// The scanner classifies per document, not per fence, and fails closed twice
// over: (1) a block that carries an `apiVersion` key plus a type field but that
// the real parser cannot model is reported, not skipped; (2) an `apiVersion`
// document carrying neither `type` nor `resourceType` is an unrecognized shape
// that is reported, never silently classified as nothing. Nix fences are scanned
// too, so a Nix-authored envelope is judged identically to a YAML or JSON one.
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

/// Whether a status mapping is abbreviated by a bare `...` elision marker that
/// is a DIRECT child of `status`. A `...` that is the value of some other key
/// (e.g. `conditions: ...`) is not whole-status elision and does not qualify.
fn status_is_elided(entries: &[Entry]) -> bool {
    entries.iter().any(|e| e.key == "...")
}

fn scan_universal_status(file: &str, content: &str) -> Vec<Violation> {
    let mut out = Vec::new();
    for block in fenced_blocks(content) {
        if !matches!(block.lang.as_str(), "yaml" | "yml" | "json" | "nix" | "") {
            continue;
        }
        if !mentions_key(&block.lines, "apiVersion") {
            continue;
        }
        let intends_envelope =
            mentions_key(&block.lines, "type") || mentions_key(&block.lines, "resourceType");
        let docs = match parse_block_docs(&block.lang, &block.lines) {
            Ok(docs) => docs,
            Err(_) => {
                // Fail closed: a block that clearly intends an envelope (an
                // apiVersion key plus a type field) but that the real parser
                // cannot model is a parser gap, not a pass.
                if intends_envelope {
                    out.push(Violation {
                        file: file.to_string(),
                        line: block.body_start + 1,
                        text: "block declares an `apiVersion` resource envelope the structural parser could not model; a lint must fail closed on an unparseable envelope, not skip it (D088/D091)"
                            .to_string(),
                    });
                }
                continue;
            }
        };
        for doc in &docs {
            let mut maps = Vec::new();
            collect_maps(doc, &mut maps);
            for m in &maps {
                if direct_child(m, "apiVersion").is_none() {
                    continue;
                }
                let has_type = direct_child(m, "type").is_some();
                let has_resource_type = direct_child(m, "resourceType").is_some();
                // Fail closed on an unrecognized envelope: an apiVersion document
                // that names neither `type` nor `resourceType` is classified as
                // exactly one thing - unrecognized - and reported, never skipped.
                if !has_type && !has_resource_type {
                    out.push(Violation {
                        file: file.to_string(),
                        line: block.body_start
                            + key_line(&block.lines, "apiVersion").unwrap_or(0)
                            + 1,
                        text: "`apiVersion` document carries neither a `type` nor a `resourceType`; a resource envelope must declare exactly one, and an unclassifiable envelope fails closed (D088/D091)"
                            .to_string(),
                    });
                    continue;
                }
                // A bundle envelope (resourceType, no type) is the compiler
                // contract with a deliberately null status: a distinct contract.
                if !has_type && has_resource_type {
                    continue;
                }
                let Some(status) = direct_child(m, "status") else {
                    continue; // spec-only fragment: no status shown.
                };
                let line = block.body_start + key_line(&block.lines, "status").unwrap_or(0) + 1;
                match &status.val {
                    // status: ... (inline elision) is a deliberate abbreviation.
                    Node::Elision => {}
                    Node::Map(entries) => {
                        if status_is_elided(entries) {
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
                    // status: null / {} / a scalar / a sequence / a placeholder
                    // / a Nix expression on a live envelope is incomplete, not
                    // abbreviated.
                    Node::Null
                    | Node::Scalar(_)
                    | Node::Seq(_)
                    | Node::Placeholder
                    | Node::Opaque => out.push(status_violation(
                        file,
                        line,
                        &["status.update", "status.resource"],
                    )),
                }
            }
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

#[test]
fn d116_exemption_binds_to_one_resource_not_the_whole_fence() {
    // The exemption must suppress only the single parsed resource that carries
    // the marker, never every violating resource in the same fence. Here one
    // fence declares the marked negative example beside an unmarked, genuinely
    // violating sibling; only the sibling must be reported.
    let marked_plus_sibling = "\
```nix
d2b.zones.dev.resources.host-teaching = {
  type = \"Host\";
  spec = {
    allowedDomains = [\"system\" \"user\"];
    # defaultUserRef intentionally omitted -> eval error
    # d2b-lint: expect-d116-eval-error
  };
};
d2b.zones.dev.resources.host-real = {
  type = \"Host\";
  spec = {
    allowedDomains = [\"system\" \"user\"];
  };
};
```";
    assert_eq!(
        scan_d116(D116_EXEMPT_FILE, marked_plus_sibling).len(),
        1,
        "the unmarked violating sibling must still be reported"
    );

    // Moving the marker onto the OTHER resource keeps exactly one report: the
    // suppression follows the marked resource, not a fixed position in the
    // fence. Combined with the both-unmarked case below (two reports), this
    // proves the binding is per-resource, not fence-wide.
    let sibling_first = "\
```nix
d2b.zones.dev.resources.host-real = {
  type = \"Host\";
  spec = {
    allowedDomains = [\"system\" \"user\"];
  };
};
d2b.zones.dev.resources.host-teaching = {
  type = \"Host\";
  spec = {
    allowedDomains = [\"system\" \"user\"];
    # defaultUserRef intentionally omitted -> eval error
    # d2b-lint: expect-d116-eval-error
  };
};
```";
    assert_eq!(
        scan_d116(D116_EXEMPT_FILE, sibling_first).len(),
        1,
        "suppression follows the marked resource regardless of order"
    );

    // With no marker at all, both violating resources are reported: the marker
    // is doing the suppression, and it suppresses exactly one.
    let both_unmarked = "\
```nix
d2b.zones.dev.resources.host-a = {
  type = \"Host\";
  spec = {
    allowedDomains = [\"system\" \"user\"];
  };
};
d2b.zones.dev.resources.host-b = {
  type = \"Host\";
  spec = {
    allowedDomains = [\"system\" \"user\"];
  };
};
```";
    assert_eq!(
        scan_d116(D116_EXEMPT_FILE, both_unmarked).len(),
        2,
        "with no marker both violating resources are reported"
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
fn universal_status_classifies_each_document_not_the_whole_fence() {
    // Classification is per document, not per fence: a recognized, complete
    // envelope in a fence must never mask an unrecognized sibling in the same
    // fence. Here a valid live envelope precedes a sibling that carries an
    // `apiVersion` but neither `type` nor `resourceType`; the sibling must
    // still be reported as unclassifiable.
    let recognized_then_unrecognized = "\
```yaml
apiVersion: resources.d2bus.org/v3
type: Widget
status:
  phase: Ready
  update:
    state: Current
  resource:
    availability: ready
---
apiVersion: resources.d2bus.org/v3
metadata:
  name: mystery
```";
    let v = scan_universal_status("f.md", recognized_then_unrecognized);
    assert_eq!(
        v.len(),
        1,
        "the unrecognized sibling must still be reported: {}",
        report("dbg", &v)
    );
    assert!(
        v[0].text.contains("neither a `type` nor a `resourceType`"),
        "the report must name the unclassifiable-envelope reason: {}",
        report("dbg", &v)
    );

    // A recognized bundle envelope (resourceType) must likewise not mask an
    // unrecognized live sibling that omits its status base.
    let bundle_then_incomplete = "\
```yaml
apiVersion: resources.d2bus.org/v3
resourceType: Role
metadata:
  name: process-controller
status: null
---
apiVersion: resources.d2bus.org/v3
type: Widget
status:
  phase: Ready
```";
    let v = scan_universal_status("f.md", bundle_then_incomplete);
    assert_eq!(
        v.len(),
        1,
        "the incomplete live sibling behind a bundle must be reported: {}",
        report("dbg", &v)
    );
    assert!(v[0].text.contains("status.update") || v[0].text.contains("status.resource"));

    // Nix fences carrying an `apiVersion` are scanned too, and each attrset is
    // classified independently. Here a complete live envelope sits beside an
    // unrecognized sibling; only the sibling is reported.
    let nix_mixed = "\
```nix
d2b.docs.recognized = {
  apiVersion = \"resources.d2bus.org/v3\";
  type = \"Widget\";
  status = {
    phase = \"Ready\";
    update = { state = \"Current\"; };
    resource = { availability = \"ready\"; };
  };
};
d2b.docs.unrecognized = {
  apiVersion = \"resources.d2bus.org/v3\";
  metadata = { name = \"mystery\"; };
};
```";
    let v = scan_universal_status("f.md", nix_mixed);
    assert_eq!(
        v.len(),
        1,
        "a Nix fence must be scanned and classified per attrset: {}",
        report("dbg", &v)
    );
    assert!(v[0].text.contains("neither a `type` nor a `resourceType`"));
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
fn parser_backed_scanners_reject_structural_bypasses() {
    // Each structural counterexample below is asserted rejected (or accepted)
    // through the public scanner, so a regression that reopens a bypass fails
    // here.

    // 1. An escaped JSON key `"ty\u0070e"` decodes to `type`, so the document is
    //    classified as a LIVE envelope and its incomplete status is flagged. The
    //    proof is the message: a status-base violation, NOT the
    //    neither-type-nor-resourceType message - i.e. the key decoded rather
    //    than staying the literal `tyu0070e` (which would misclassify).
    let escaped_key = "```json\n{\n  \"apiVersion\": \"resources.d2bus.org/v3\",\n  \"ty\\u0070e\": \"Widget\",\n  \"status\": { \"phase\": \"Ready\" }\n}\n```";
    let v = scan_universal_status("f.md", escaped_key);
    assert_eq!(v.len(), 1, "{}", report("escaped-key", &v));
    assert!(
        v[0].text.contains("universal status base"),
        "the escaped `ty\\u0070e` key must decode to `type` and classify a LIVE envelope: {}",
        v[0].text
    );

    // 2. A Nix `rec { ... }` resource is parsed, not discarded. A rec-set live
    //    envelope with an incomplete status is flagged.
    let rec_nix = "```nix\nrec {\n  apiVersion = \"resources.d2bus.org/v3\";\n  type = \"Widget\";\n  status = { phase = \"Ready\"; };\n}\n```";
    let v = scan_universal_status("f.md", rec_nix);
    assert_eq!(v.len(), 1, "{}", report("rec-nix", &v));
    assert!(v[0].text.contains("universal status base"));

    // Nix expressions that legally wrap or contain an attrset must never hide a
    // violating envelope from the scanner. These fixtures cover the supported
    // wrapper class, not just the two forms that originally exposed the gap.
    let incomplete = r#"{
  apiVersion = "resources.d2bus.org/v3";
  type = "Widget";
  status = { phase = "Ready"; };
}"#;
    let nix_wrappers = [
        ("with", format!("with {{}}; {incomplete}")),
        (
            "with-bound-result",
            format!("with {{ resource = {incomplete}; }}; resource"),
        ),
        (
            "nested-let",
            format!("let a = 1; in let b = 2; in {incomplete}"),
        ),
        (
            "let-bound-result",
            format!("let resource = {incomplete}; in resource"),
        ),
        ("rec", incomplete.replacen('{', "rec {", 1)),
        ("parenthesized", format!("({incomplete})")),
        (
            "if-then-branch",
            format!("if true then {incomplete} else {{}}"),
        ),
        (
            "if-else-branch",
            format!("if false then {{}} else {incomplete}"),
        ),
        ("list-item", format!("[ {incomplete} ]")),
        ("assert-body", format!("assert true; {incomplete}")),
        ("lambda-body", format!("argument: {incomplete}")),
        ("binary-operand", format!("{{}} // {incomplete}")),
        (
            "selected-attribute",
            format!("({{ selected = {incomplete}; }}).selected"),
        ),
        ("legacy-let-body", format!("let {{ body = {incomplete}; }}")),
        (
            "string-interpolation",
            r#"{
  apiVersion = "resources.d2bus.org/v3";
  type = "Widget-${variant}";
  status = { phase = "Ready"; };
}"#
            .to_string(),
        ),
    ];
    for (shape, expr) in nix_wrappers {
        let fixture = format!("```nix\n{expr}\n```");
        let v = scan_universal_status("f.md", &fixture);
        assert_eq!(
            v.len(),
            1,
            "{shape} must expose its violating resource: {}",
            report(shape, &v)
        );
        assert!(
            v[0].text.contains("universal status base"),
            "{shape} must reach the resource rather than a parser-gap fallback: {}",
            v[0].text
        );
    }

    // Function application may return an attrset. The structural model does
    // not evaluate the function, but it exposes every literal argument map as a
    // candidate rather than collapsing the application to an unchecked opaque
    // node.
    let application = format!("```nix\nidentity {incomplete}\n```");
    let v = scan_universal_status("f.md", &application);
    assert_eq!(
        v.len(),
        1,
        "application must fail closed: {}",
        report("application", &v)
    );
    assert!(
        v[0].text.contains("universal status base"),
        "application must expose the literal resource argument: {}",
        v[0].text
    );

    // 3. A YAML anchor + `<<` merge is folded. A status assembled from a merge
    //    that supplies only `phase` is still missing update/resource (flagged);
    //    a merge that supplies the whole base is clean (proving the merge is
    //    modeled, not mishandled or dropped).
    let merge_incomplete = "```yaml\napiVersion: resources.d2bus.org/v3\ntype: Widget\n_base: &b\n  phase: Ready\nstatus:\n  <<: *b\n```";
    assert!(
        !scan_universal_status("f.md", merge_incomplete).is_empty(),
        "a status whose merged base lacks update/resource must be flagged"
    );
    let merge_complete = "```yaml\napiVersion: resources.d2bus.org/v3\ntype: Widget\n_base: &b\n  phase: Ready\n  update: { state: Current }\n  resource: { availability: ready }\nstatus:\n  <<: *b\n```";
    assert!(
        scan_universal_status("f.md", merge_complete).is_empty(),
        "a status whose complete base is merged in must be accepted: {}",
        report(
            "merge-complete",
            &scan_universal_status("f.md", merge_complete)
        )
    );

    // 4. An apiVersion document carrying NEITHER type NOR resourceType is
    //    classified as exactly one thing - unrecognized - and fails closed.
    let neither = "```yaml\napiVersion: resources.d2bus.org/v3\nmetadata:\n  name: x\nstatus:\n  phase: Ready\n```";
    let v = scan_universal_status("f.md", neither);
    assert_eq!(v.len(), 1, "{}", report("neither", &v));
    assert!(v[0].text.contains("neither a `type` nor a `resourceType`"));

    // 5. Two envelopes in ONE fence, only the second invalid: classification is
    //    per document, so exactly the second is flagged - no block-wide flag
    //    masks a sibling.
    let two_docs = "```yaml\napiVersion: resources.d2bus.org/v3\ntype: Widget\nstatus:\n  phase: Ready\n  update: { state: Current }\n  resource: { availability: ready }\n---\napiVersion: resources.d2bus.org/v3\ntype: Gadget\nstatus:\n  phase: Ready\n```";
    let v = scan_universal_status("f.md", two_docs);
    assert_eq!(
        v.len(),
        1,
        "only the second (incomplete) envelope is flagged: {}",
        report("two-docs", &v)
    );
    assert!(v[0].text.contains("universal status base"));

    // 6. `conditions: ...` nested under status is a `...` VALUE on some other
    //    key, not a direct `...` marker child of status, so it does NOT
    //    abbreviate the whole status; the missing base is still flagged.
    let nested_value_elision = envelope_with_status("  phase: Ready\n  conditions: ...");
    let v = scan_universal_status("f.md", &nested_value_elision);
    assert_eq!(v.len(), 1, "{}", report("nested-value-elision", &v));
    assert!(v[0].text.contains("universal status base"));
}

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
