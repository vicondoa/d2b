#!/usr/bin/env bash
# tests/tools/security-scan.sh - identifier-in-log redaction scan (issue #586)
#
# Deterministic grep-based scanner for the ADR 0010/0028 redaction
# discipline: no opaque correlation identifier is written into a log.
# This is the in-tree first implementation of the security scan that
# issue #586 makes a required check; the external scanner engine that
# produced the original findings remains owner-confirmed, and this job is
# the pinned deterministic rule until the owner names that engine.
#
# Rule (pinned):
#   A changed line of in-tree Rust source that both
#     - emits a log record: tracing::trace!/debug!/info!/warn!/error!,
#       log::trace!/debug!/info!/warn!/error!, the bare trace!/debug!/
#       info!/warn!/error! forms, or println!/eprintln!/print!/eprint!;
#       and
#     - references one of the pinned opaque correlation identifiers
#       (root_invocation_id, invocation_id, operation_id, session_id,
#       audit_id, trace_id - the ADR 0010 audit invocation identifiers,
#       the ADR 0032 admission audit ids, the ADR 0038 opaque correlation
#       ids, and the audit "opaque operation/trace identifier" fields)
#   is a finding and fails the scan. Identifiers that the tree
#   legitimately logs (vm_id, role_id, request_id, stream_id) are not in
#   the pinned set.
#
# Scope (pinned):
#   - Changed-line mode (D2B_SCAN_BASE_SHA set): the added lines of the
#     diff from the merge base with D2B_SCAN_BASE_SHA to HEAD (the
#     workflow passes github.event.pull_request.base.sha on pull requests
#     and github.event.before on pushes).
#   - Full-tree mode (D2B_SCAN_BASE_SHA unset): every tracked .rs file
#     (the local runnable and the workflow_dispatch default).
#   - In-tree Rust only: the vendored pkgs/ tree is third-party code and
#     is not an ADR 0010/0028 surface.
#   - The scan is line-based and conservative: a pinned identifier
#     anywhere on a log-emitting line is a finding, including a mention
#     in message text. A false positive is resolved by rewording the
#     line, never by allowlisting.
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
pattern="(${log_macro}).*${identifier}|${identifier}.*(${log_macro})"

base="${D2B_SCAN_BASE_SHA:-}"
if [[ -n "$base" ]]; then
  if ! git rev-parse --verify -q "$base" >/dev/null; then
    echo "security-scan: base sha $base is not present in this checkout" >&2
    exit 2
  fi
  added_lines="$(
    git diff --diff-filter=AM --unified=0 "$base"...HEAD \
      -- '*.rs' ':(exclude)pkgs/**' |
      awk '
        /^diff --git / { file = $4; sub(/^b\//, "", file); next }
        /^\+\+\+ / { next }
        /^@@ / {
          if (match($0, /\+[0-9]+/)) {
            newline = substr($0, RSTART + 1, RLENGTH - 1) + 0
          }
          next
        }
        /^\+/ {
          printf "%s:%d:%s\n", file, newline, substr($0, 2)
          newline++
          next
        }
      ' |
      grep -E "$pattern" || true
  )"
  scanned="changed in-tree Rust lines since $base"
else
  added_lines="$(
    git ls-files '*.rs' ':(exclude)pkgs/**' |
      xargs -r grep -nE "$pattern" 2>/dev/null || true
  )"
  scanned="tracked in-tree Rust files"
fi

if [[ -n "$added_lines" ]]; then
  echo "security-scan: FAIL - identifier-in-log redaction finding(s) on $scanned:" >&2
  printf '%s\n' "$added_lines" >&2
  echo "security-scan: the ADR 0010/0028 discipline forbids writing an opaque correlation identifier into a log; reword the line (rule pinned in docs/contributing/workflow.md)" >&2
  exit 1
fi

echo "security-scan: clean ($scanned)"