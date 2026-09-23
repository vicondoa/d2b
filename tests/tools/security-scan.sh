#!/usr/bin/env bash
# tests/tools/security-scan.sh - identifier-in-log redaction scan (issue #586)
#
# Deterministic scanner for the ADR 0010/0028 redaction discipline: no
# opaque correlation identifier is written into a log. This is the
# in-tree first implementation of the security scan that issue #586 pins;
# the external scanner engine that produced the original findings remains
# owner-confirmed, and this job is the pinned deterministic rule until the
# owner names that engine.
#
# Rule (pinned):
#   A log-emitting macro invocation of in-tree Rust source that references
#   one of the pinned opaque correlation identifiers is a finding and
#   fails the scan. The rule is evaluated against the WHOLE macro
#   invocation: the scanner buffers from a log-macro opening line until
#   the invocation's closing `);` and matches the identifier anywhere in
#   the buffered record, including a mention in message text (a
#   rustfmt-conventional multi-line invocation - macro opened on one
#   line, fields on continuation lines - is therefore a finding when any
#   part of the record references a pinned identifier). The buffered
#   record is scanned literal-aware: string/char-literal/block-comment
#   state is carried across the record's lines, so a `)` inside a
#   multi-line string or char literal is never mistaken for the
#   invocation's closing delimiter. A false positive is resolved by
#   rewording the record, never by allowlisting.
#
# Scope (pinned):
#   - Changed-line mode (D2B_SCAN_BASE_SHA set): the added lines of the
#     diff from the merge base with D2B_SCAN_BASE_SHA against the working
#     tree (on pull requests the workflow passes
#     github.event.pull_request.base.sha; on pushes github.event.before).
#     Diffing the merge base against the working tree means a local run
#     also sees uncommitted lines. An all-zeros base (a first push) is
#     rejected explicitly: a changed-line scan against it cannot prove
#     the rule, so it exits 2 and is never reported clean. If the diff
#     itself fails, the scan exits 2 - a scan that could not run is never
#     reported clean.
#   - Full-tree mode (D2B_SCAN_BASE_SHA unset): every tracked in-tree
#     .rs file (the local runnable and the workflow_dispatch default).
#   - In-tree Rust only: the vendored pkgs/ tree is third-party code and
#     is not an ADR 0010/0028 surface.
#   - Renames and copies are scanned: the changed-line diff keeps R and C
#     statuses (--diff-filter=ACMR), so a violation that arrives with a
#     rename/copy is still a finding.
#
# Exit status: 0 = no findings, 1 = findings, 2 = the scan could not run.
set -euo pipefail

root="${D2B_REPO_ROOT:-}"
if [[ -z "$root" ]]; then
  root="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"
fi
cd "$root"

log_macro='(tracing|log)::(trace|debug|info|warn|error)!|(^|[^[:alnum:]_])(trace|debug|info|warn|error)!|(eprintln|println|eprint|print)!'
identifier='(root_invocation_id|invocation_id|operation_id|session_id|audit_id|trace_id)'

# scan_file <file> <added-csv|->: evaluate the whole-invocation rule on
# one in-tree Rust file. Prints `file:startline:record` for each finding.
# In changed-line mode (added-csv non-empty) a record is a finding only
# when its line span intersects the added set; in full-tree mode
# (added-csv empty) every matching record is a finding.
scan_file() {
  local file="$1" added_csv="$2"
  awk -v file="$file" -v added_csv="$added_csv" -v log_macro="$log_macro" -v identifier="$identifier" '
    # char_literal(s, i): s[i] is a quote that starts a char literal
    # (not a lifetime) when a closing quote follows on the same line.
    function char_literal(s, i,    j, n, c, esc) {
      n = length(s)
      c = substr(s, i + 1, 1)
      if (c == "\\") {
        j = i + 2
        esc = 0
        while (j <= n) {
          c = substr(s, j, 1)
          if (esc) { esc = 0; j++; continue }
          if (c == "\\") { esc = 1; j++; continue }
          if (c == "\047") return 1
          j++
        }
        return 0
      }
      if (c == "") return 0
      return (substr(s, i + 2, 1) == "\047") ? 1 : 0
    }
    # strip_strings(s): per-line stripper with fresh literal state.
    # Used ONLY for the opening-line decision (does this line open a
    # log-macro invocation?). Strings, char literals, block comments,
    # and line comments are blanked; lifetimes stay as code.
    function strip_strings(s,    out, i, n, c, in_str, in_char, in_block, esc) {
      out = ""
      n = length(s)
      in_str = 0
      in_char = 0
      in_block = 0
      for (i = 1; i <= n; i++) {
        c = substr(s, i, 1)
        if (in_str) {
          if (esc) { esc = 0; out = out " "; continue }
          if (c == "\\") { esc = 1; out = out " "; continue }
          if (c == "\"") { in_str = 0; out = out " "; continue }
          out = out " "
        } else if (in_char) {
          if (esc) { esc = 0; out = out " "; continue }
          if (c == "\\") { esc = 1; out = out " "; continue }
          if (c == "\047") { in_char = 0; out = out " "; continue }
          out = out " "
        } else if (in_block) {
          if (c == "*" && substr(s, i + 1, 1) == "/") { i++; in_block = 0; out = out "  " }
          else out = out " "
        } else {
          if (c == "\"") { in_str = 1; out = out " "; continue }
          if (c == "\047") {
            if (char_literal(s, i)) { in_char = 1; out = out " "; continue }
            out = out c
            continue
          }
          if (c == "/" && substr(s, i + 1, 1) == "*") { i++; in_block = 1; out = out "  "; continue }
          if (c == "/" && substr(s, i + 1, 1) == "/") break
          out = out c
        }
      }
      return out
    }
    function net_parens(s,    t, i, n, d, c) {
      t = strip_strings(s)
      d = 0
      n = length(t)
      for (i = 1; i <= n; i++) {
        c = substr(t, i, 1)
        if (c == "(") d++
        else if (c == ")") d--
      }
      return d
    }
    # Cross-line literal state for the buffered-record scanner: string,
    # char-literal, and block-comment state is carried across the record
    # lines, so a ")" inside a multi-line string or char literal is never
    # mistaken for the closing delimiter of the invocation.
    function reset_literal_state(   ) {
      s_str = 0
      s_char = 0
      s_block = 0
      s_esc = 0
    }
    function strip_record_line(s,    out, i, n, c) {
      out = ""
      n = length(s)
      for (i = 1; i <= n; i++) {
        c = substr(s, i, 1)
        if (s_str) {
          if (s_esc) { s_esc = 0; out = out " "; continue }
          if (c == "\\") { s_esc = 1; out = out " "; continue }
          if (c == "\"") { s_str = 0; out = out " "; continue }
          out = out " "
        } else if (s_char) {
          if (s_esc) { s_esc = 0; out = out " "; continue }
          if (c == "\\") { s_esc = 1; out = out " "; continue }
          if (c == "\047") { s_char = 0; out = out " "; continue }
          out = out " "
        } else if (s_block) {
          if (c == "*" && substr(s, i + 1, 1) == "/") { i++; s_block = 0; out = out "  " }
          else out = out " "
        } else {
          if (c == "\"") { s_str = 1; out = out " "; continue }
          if (c == "\047") {
            if (char_literal(s, i)) { s_char = 1; out = out " "; continue }
            out = out c
            continue
          }
          if (c == "/" && substr(s, i + 1, 1) == "*") { i++; s_block = 1; out = out "  "; continue }
          if (c == "/" && substr(s, i + 1, 1) == "/") break
          out = out c
        }
      }
      return out
    }
    function net_parens_stateful(s,    t, i, n, d, c) {
      t = strip_record_line(s)
      d = 0
      n = length(t)
      for (i = 1; i <= n; i++) {
        c = substr(t, i, 1)
        if (c == "(") d++
        else if (c == ")") d--
      }
      return d
    }
    function intersects(s, e,    ln) {
      if (added_csv == "") return 1
      for (ln = s; ln <= e; ln++)
        if (added[ln]) return 1
      return 0
    }
    function check(s, e, rec) {
      if ((rec ~ identifier) && intersects(s, e))
        printf "%s:%d:%s\n", file, s, rec
    }
    BEGIN {
      if (added_csv != "") {
        n = split(added_csv, la, ",")
        for (i = 1; i <= n; i++) added[la[i] + 0] = 1
      }
      in_record = 0
      record = ""
      depth = 0
      record_start = 0
      reset_literal_state()
    }
    {
      if (in_record) {
        record = record "\n" $0
        # Continuation lines are stripped with the cross-line literal
        # state carried from the earlier lines of the record, so a ")" inside
        # a multi-line string/char/block-comment is never counted as a
        # paren; the record ends at the real closing delimiter of the
        # invocation (net depth back to 0).
        depth += net_parens_stateful($0)
        if (depth <= 0) {
          check(record_start, NR, record)
          in_record = 0
          record = ""
          depth = 0
          reset_literal_state()
        }
        next
      }
      if (strip_strings($0) ~ log_macro) {
        d = net_parens($0)
        if (d > 0) {
          in_record = 1
          record = $0
          record_start = NR
          depth = d
          # Seed the cross-line literal state from the opening line too:
          # a string/char/block-comment opened on the opening line
          # continues into the continuation lines of the record.
          reset_literal_state()
          strip_record_line($0)
        } else {
          check(NR, NR, $0)
        }
        next
      }
    }
    END {
      if (in_record) check(record_start, NR, record)
    }
  ' "$file"
}

findings=""
scanned=""

base="${D2B_SCAN_BASE_SHA:-}"
if [[ -n "$base" ]]; then
  # Reject the all-zeros base explicitly: git rev-parse -q accepts it,
  # but a changed-line scan against an all-zeros base cannot prove the
  # rule, so it must never be reported clean.
  if [[ "$base" =~ ^0+$ ]]; then
    echo "security-scan: base sha $base is all zeros (first-push / uninitialized base); a changed-line scan cannot prove the rule against it, refusing to report clean" >&2
    exit 2
  fi
  if ! git rev-parse --verify -q "$base" >/dev/null; then
    echo "security-scan: base sha $base is not present in this checkout" >&2
    exit 2
  fi
  merge_base="$(git merge-base "$base" HEAD)" || {
    echo "security-scan: no merge base between $base and HEAD; the changed-line scan cannot be defined" >&2
    exit 2
  }
  # Capture the diff pipeline status: a failed diff is a scan that could
  # not run (exit 2), never a clean report. git diff exits 0 (no
  # differences) or 1 (differences exist) on success; anything >= 2 is a
  # real failure.
  diff_file="$(mktemp)"
  trap 'rm -f "$diff_file"' EXIT
  set +e
  git diff --diff-filter=ACMR -M -C --unified=0 "$merge_base" \
    -- '*.rs' ':(exclude)pkgs/**' > "$diff_file"
  diff_status=$?
  set -e
  if (( diff_status >= 2 )); then
    echo "security-scan: the changed-line diff could not run (git diff exited $diff_status); a scan that could not run is never reported clean" >&2
    exit 2
  fi
  added_rows="$(
    awk '
      /^diff --git / { file = $4; sub(/^b\//, "", file); next }
      /^\+\+\+ / { next }
      /^@@ / {
        if (match($0, /\+[0-9]+/)) newline = substr($0, RSTART + 1, RLENGTH - 1) + 0
        next
      }
      /^\+/ { printf "%s:%d\n", file, newline; newline++; next }
    ' "$diff_file"
  )"
  rm -f "$diff_file"
  trap - EXIT
  if [[ -n "$added_rows" ]]; then
    while IFS= read -r file; do
      added_csv="$(
        printf '%s\n' "$added_rows" |
          awk -F: -v f="$file" '$1 == f { if (cnt++) printf ","; printf "%s", $2 }'
      )"
      file_findings="$(scan_file "$file" "$added_csv")"
      findings="${findings}${file_findings}"
    done < <(printf '%s\n' "$added_rows" | cut -d: -f1 | sort -u)
  fi
  scanned="changed in-tree Rust lines since the merge base $merge_base"
else
  # Capture the enumeration and check its status: a failed enumeration
  # (e.g. outside a git checkout) must be a scan that could not run
  # (exit 2), never a clean report. Feeding the loop from a process
  # substitution would silently scan zero files when ls-files fails and
  # report clean - a green verdict from a scan that never ran.
  ls_file="$(mktemp)"
  trap 'rm -f "$ls_file"' EXIT
  set +e
  git ls-files '*.rs' ':(exclude)pkgs/**' > "$ls_file"
  ls_status=$?
  set -e
  if (( ls_status != 0 )); then
    echo "security-scan: the in-tree Rust file enumeration could not run (git ls-files exited $ls_status); a scan that could not run is never reported clean" >&2
    rm -f "$ls_file"
    trap - EXIT
    exit 2
  fi
  while IFS= read -r file; do
    file_findings="$(scan_file "$file" "")"
    findings="${findings}${file_findings}"
  done < "$ls_file"
  rm -f "$ls_file"
  trap - EXIT
  scanned="tracked in-tree Rust files"
fi

if [[ -n "$findings" ]]; then
  echo "security-scan: FAIL - identifier-in-log redaction finding(s) on $scanned:" >&2
  printf '%s\n' "$findings" >&2
  echo "security-scan: the ADR 0010/0028 discipline forbids writing an opaque correlation identifier into a log; redact the identifier from the record at source (rule pinned in docs/contributing/workflow.md)" >&2
  exit 1
fi

echo "security-scan: clean ($scanned)"