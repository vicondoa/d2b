//! ADR-046 resource-envelope structural lints.
//!
//! Two ADR-046 decisions have, until now, had no mechanical enforcement and
//! have each survived multiple manual sweeps: D116 (a Host or Guest that admits
//! the `user` domain must name a `defaultUserRef`) and the universal status
//! contract (D088 three-layer status base + D091 `status.update` currency
//! object). Manual sweeps keep missing siblings in files never previously
//! cited; a lint does not.
//!
//! Both scanners operate on the committed `docs/specs/**` tree, never on a
//! fixture, and both are careful to distinguish a *complete envelope* - the
//! authoring context each decision actually governs - from a *focused fragment*
//! or a *shorthand schema table*, which are deliberately exempt:
//!
//! * A **complete envelope** is a fenced code block whose top level carries the
//!   full object frame the decision governs. For D116 that is a Host/Guest
//!   `type` declaration paired with an `allowedDomains` list; for the status
//!   contract it is a YAML document carrying all of `apiVersion`, `type`,
//!   `metadata`, `spec`, and a non-abbreviated `status`.
//! * A **focused fragment** shows only part of an object - a lone `status:`
//!   snippet with no `apiVersion`, a Host `spec` with no `allowedDomains`, or a
//!   `status:` whose body is elided with a bare `...`. A fragment is exempt
//!   because the omitted keys are deliberately out of frame, not missing.
//! * A **shorthand schema table** is a Markdown `| ... |` table describing a
//!   schema in prose. Tables are never inside a code fence, so neither scanner
//!   ever reads one.
//!
//! Each scanner is exercised by a planted-violation fixture and a clean-input
//! fixture, then run over the real tree. When the real tree is dirty the gate
//! reports every offending block with a file and line so an author can act on
//! it directly; a stricter lint that surfaces a real `docs/**` violation is the
//! correct outcome, never a reason to weaken the grammar.

use std::path::{Path, PathBuf};

use d2b_contract_tests::repo_root;
use regex::Regex;

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
// never a resource-authoring context. A block carries its language tag (so the
// status scanner can restrict to YAML) and the absolute 0-based index of its
// first body line (so a violation can be reported at a real file line).
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
// D116 - a Host or Guest admitting the `user` domain must name a defaultUserRef.
//
// The decision register (D116) freezes the superset invariant: `defaultUserRef`
// is present whenever `allowedDomains` contains `user`, independent of the
// default domain. A `defaultUserRef` that is present but `null` does not satisfy
// it - a null ref is the absence of a ref. The grammar is language-neutral so a
// YAML, JSON, or Nix authoring block is judged identically:
//
//   * host_guest:  a `type` field whose value is `Host` or `Guest`
//                  (`type: Host`, `"type": "Host"`, or `type = "Host"`).
//   * user domain: an `allowedDomains` list whose tokens include exactly `user`.
//   * ref present: a `defaultUserRef` field with a non-null, non-empty value.
//
// A block is judged only when it is a Host/Guest *and* declares `allowedDomains`
// - i.e. an execution-policy authoring context. A block that shows a Host spec
// without `allowedDomains`, or `allowedDomains` without a Host/Guest `type`, is
// a focused fragment and is not judged.
//
// A spec may also show an *intentional negative example* - an authored shape
// that is meant to be rejected, e.g. the block that demonstrates the D116
// eval-time failure by deliberately omitting `defaultUserRef`. Such a block is
// structurally a violation, but flagging it would push the author to "fix"
// correct teaching content. It is exempted only when it carries the explicit,
// greppable marker comment `d2b-lint: expect-d116-...` inside the fence (any
// comment line mentioning both `d2b-lint` and `d116`). This mirrors the
// universal-status `...` elision exemption: an unambiguous authoring signal,
// never English sentiment, so a real declaration - which never carries such a
// self-incriminating marker - stays flagged.
// ---------------------------------------------------------------------------

fn d116_host_guest() -> Regex {
    // Not anchored to the line start: a single-line JSON object packs `type`
    // mid-line. The leading `\b` keeps a longer word such as `subtype` from
    // matching, and the trailing `\b` after Host/Guest keeps `HostGroup` out.
    Regex::new(r#"\btype"?\s*[:=]\s*"?(?:Host|Guest)\b"#).expect("valid D116 host/guest regex")
}

fn d116_default_user_ref() -> Regex {
    // `defaultUserRef` may appear mid-line (JSON packs several keys per line),
    // so this is not anchored to the line start; a `\b` prevents matching a
    // longer identifier that merely ends in `defaultUserRef`. The value runs to
    // the next structural delimiter (comma, brace, semicolon, comment, EOL).
    Regex::new(r#"\bdefaultUserRef"?\s*[:=]\s*(?P<val>[^,;}#\n]*)"#)
        .expect("valid D116 defaultUserRef regex")
}

/// Whether the `allowedDomains` list on `line` contains the exact `user` token.
/// The bracketed list is tokenized on quotes, commas, and whitespace so
/// `[system, user]`, `["system" "user"]`, and `["system", "user"]` are read
/// identically, and a longer word that merely contains `user` is not a match.
fn allowed_domains_has_user(line: &str) -> bool {
    let Some(open) = line.find('[') else {
        return false;
    };
    let rest = &line[open + 1..];
    let close = rest.find(']').unwrap_or(rest.len());
    rest[..close]
        .split(|c: char| c == ',' || c == '"' || c == '\'' || c.is_whitespace())
        .any(|tok| tok == "user")
}

/// Whether `defaultUserRef`'s captured value is a real reference: present and
/// not `null`. A missing field and a `null` value are both unsatisfied.
fn default_user_ref_is_set(block: &[&str]) -> bool {
    let re = d116_default_user_ref();
    let joined = block.join("\n");
    match re.captures(&joined) {
        None => false,
        Some(caps) => {
            let val = caps["val"]
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .trim();
            !val.is_empty() && val != "null"
        }
    }
}

/// Whether a judged block is an explicitly marked intentional negative example.
/// The signal is a greppable marker comment naming both `d2b-lint` and `d116`
/// (case-insensitive), e.g. `# d2b-lint: expect-d116-eval-error`. Only a comment
/// line (leading `#`, `//`, or Nix `#`) may carry it, so a stray mention in a
/// string value cannot suppress a real violation.
fn d116_marked_negative_example(block: &[&str]) -> bool {
    block.iter().any(|l| {
        let t = l.trim_start();
        let is_comment = t.starts_with('#') || t.starts_with("//");
        if !is_comment {
            return false;
        }
        let lower = t.to_ascii_lowercase();
        lower.contains("d2b-lint") && lower.contains("d116")
    })
}

fn scan_d116(file: &str, content: &str) -> Vec<Violation> {
    let host_guest = d116_host_guest();
    let mut out = Vec::new();
    for block in fenced_blocks(content) {
        let body = block.lines.join("\n");
        if !host_guest.is_match(&body) {
            continue;
        }
        // The `allowedDomains` line, if any, is the one authoring context that
        // makes `defaultUserRef` mandatory. No such line -> focused fragment.
        let Some((offset, dline)) = block
            .lines
            .iter()
            .enumerate()
            .find(|(_, l)| l.contains("allowedDomains"))
        else {
            continue;
        };
        if !allowed_domains_has_user(dline) {
            continue;
        }
        if default_user_ref_is_set(&block.lines) {
            continue;
        }
        // An explicitly marked intentional negative example is documenting the
        // rejection, not declaring a resource; do not flag it.
        if d116_marked_negative_example(&block.lines) {
            continue;
        }
        out.push(Violation {
            file: file.to_string(),
            line: block.body_start + offset + 1,
            text: "Host/Guest with `user` in allowedDomains is missing a non-null `defaultUserRef` (D116)"
                .to_string(),
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Universal status contract - every complete envelope carries the D088 base
// plus `status.update` (D091) and `status.resource` (D088 Layer 2).
//
// A complete envelope is a YAML document carrying all of `apiVersion`, `type`,
// `metadata`, `spec`, and `status` at the document's top level, whose `status`
// subtree is not abbreviated with a bare `...` elision line. For such an
// envelope the `status` subtree must carry, among its direct children, both
// `update` (the universal currency object) and `resource` (the ResourceType
// Layer-2 base). A subtree that carries `credential`, `device`, or any other
// ResourceType-specific key in place of `resource` is a violation: the Layer-2
// key is frozen as `resource`.
//
// Exemptions fall out of the grammar rather than an allowlist: a lone `status:`
// fragment has no `apiVersion` so it is not a complete envelope; an envelope
// whose `status` body is elided with `...` is deliberately abbreviated; a
// Markdown schema table is not inside a code fence at all.
// ---------------------------------------------------------------------------

/// The top-level (zero-indent) YAML keys present in one document.
fn top_level_keys(doc: &[&str]) -> Vec<String> {
    let key = Regex::new(r"^(?P<k>[A-Za-z][A-Za-z0-9_]*):").expect("valid top-level key regex");
    doc.iter()
        .filter(|l| indent(l) == 0)
        .filter_map(|l| key.captures(l).map(|c| c["k"].to_string()))
        .collect()
}

/// The body lines of the top-level `status:` block: every line after the
/// `status:` key up to the next zero-indent key or document end.
fn status_body<'a>(doc: &[&'a str]) -> Option<(usize, Vec<&'a str>)> {
    let start = doc
        .iter()
        .position(|l| indent(l) == 0 && l.trim_end() == "status:")?;
    let mut body = Vec::new();
    for &line in &doc[start + 1..] {
        if !line.trim().is_empty() && indent(line) == 0 {
            break;
        }
        body.push(line);
    }
    Some((start, body))
}

/// The direct-child keys of a `status:` block: the keys at the minimal indent
/// among the block's non-blank lines.
fn status_child_keys(body: &[&str]) -> Vec<String> {
    let child_indent = body
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| indent(l))
        .min();
    let Some(child_indent) = child_indent else {
        return Vec::new();
    };
    let key = Regex::new(r"^\s*(?P<k>[A-Za-z][A-Za-z0-9_]*):").expect("valid child key regex");
    body.iter()
        .filter(|l| !l.trim().is_empty() && indent(l) == child_indent)
        .filter_map(|l| key.captures(l).map(|c| c["k"].to_string()))
        .collect()
}

/// Split a block body into YAML sub-documents on a bare `---` separator so a
/// fence concatenating two envelopes is judged one document at a time.
fn yaml_documents<'a>(block: &[&'a str]) -> Vec<(usize, Vec<&'a str>)> {
    let mut docs = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut cur_start = 0;
    for (i, &line) in block.iter().enumerate() {
        if line.trim() == "---" {
            if !cur.is_empty() {
                docs.push((cur_start, std::mem::take(&mut cur)));
            }
            cur_start = i + 1;
            continue;
        }
        if cur.is_empty() {
            cur_start = i;
        }
        cur.push(line);
    }
    if !cur.is_empty() {
        docs.push((cur_start, cur));
    }
    docs
}

fn scan_universal_status(file: &str, content: &str) -> Vec<Violation> {
    const REQUIRED: [&str; 5] = ["apiVersion", "type", "metadata", "spec", "status"];
    let mut out = Vec::new();
    for block in fenced_blocks(content) {
        if !(block.lang == "yaml" || block.lang == "yml") {
            continue;
        }
        for (doc_offset, doc) in yaml_documents(&block.lines) {
            let keys = top_level_keys(&doc);
            if !REQUIRED.iter().all(|r| keys.iter().any(|k| k == r)) {
                continue;
            }
            let Some((status_pos, body)) = status_body(&doc) else {
                continue;
            };
            // A `status:` body elided with a bare `...` line is deliberately
            // abbreviated - the author is showing the frame, not the contents.
            if body.iter().any(|l| l.trim() == "...") {
                continue;
            }
            let children = status_child_keys(&body);
            let mut missing = Vec::new();
            if !children.iter().any(|k| k == "update") {
                missing.push("status.update");
            }
            if !children.iter().any(|k| k == "resource") {
                missing.push("status.resource");
            }
            if missing.is_empty() {
                continue;
            }
            let line = block.body_start + doc_offset + status_pos + 1;
            out.push(Violation {
                file: file.to_string(),
                line,
                text: format!(
                    "complete resource envelope is missing {} (D088/D091 universal status base)",
                    missing.join(" and ")
                ),
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
fn d116_accepts_clean_and_exempt_shapes() {
    // A real ref satisfies the invariant (YAML, JSON single-line, and Nix).
    // The JSON case packs `type` and `defaultUserRef` mid-line, which the
    // scanner must still read.
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

    // An explicitly marked intentional negative example (the D116 eval-error
    // teaching block) is exempt: it documents the rejection, and forcing a ref
    // into it would corrupt correct content.
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
        scan_d116("f.md", marked).is_empty(),
        "marked negative example"
    );

    // The SAME shape without the marker is still a violation: a real
    // declaration never carries the suppression comment, so detection of
    // genuine misses is not weakened.
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
        scan_d116("f.md", unmarked).len(),
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
fn universal_status_accepts_complete_fragment_and_abbreviated_shapes() {
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

    // An abbreviated status body (bare `...`) is deliberately elided.
    let abbreviated = envelope_with_status("  observedGeneration: 1\n  phase: Ready\n  ...");
    assert!(scan_universal_status("f.md", &abbreviated).is_empty());

    // A spec-only envelope (no status) is not a complete-status context.
    let spec_only = "```yaml\napiVersion: resources.d2bus.org/v3\ntype: Widget\nmetadata:\n  name: x\nspec:\n  providerRef: Provider/p\n```";
    assert!(scan_universal_status("f.md", spec_only).is_empty());

    // A non-YAML fence is never a YAML envelope.
    let json = "```json\n{ \"apiVersion\": \"resources.d2bus.org/v3\", \"type\": \"Widget\", \"status\": {} }\n```";
    assert!(scan_universal_status("f.md", json).is_empty());
}

#[test]
fn universal_status_ignores_prose_field_path_references() {
    // Legitimate explanatory prose references a status field path under the
    // spec's documented `status.<field>` -> `status.resource.<field>` mapping
    // convention. These are correct content, not resource-envelope examples,
    // and MUST NOT be flagged: the scanner only reads fenced YAML documents,
    // and prose lacks the five top-level envelope keys regardless.
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
    // is scanned and passes, and the prose reference is still ignored, so the
    // document as a whole is clean.
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
    // not a fenced YAML envelope, so it is never scanned.
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
