#!/usr/bin/env bash
# Asserts that ADR 0015 (daemon-only clean break) exists, carries the
# expected status/wave header, and is cross-referenced from AGENTS.md.
# Layer-1, eval-only (no flake build, no daemon, no host state).

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

adr="$ROOT/docs/adr/0015-daemon-only-clean-break.md"
agents="$ROOT/AGENTS.md"
adr_index="$ROOT/docs/adr/README.md"

fail() {
  printf 'tests/adr-0015-presence-eval: %s\n' "$*" >&2
  exit 1
}

[ -f "$adr" ] || fail "missing $adr"
[ -f "$agents" ] || fail "missing $agents"
[ -f "$adr_index" ] || fail "missing $adr_index"

# Header invariants.
grep -qE '^# 0015\. ' "$adr" \
  || fail "$adr missing canonical '# 0015.' title line"
grep -qE '^- Status: Accepted$' "$adr" \
  || fail "$adr missing 'Status: Accepted' header"
grep -qE '^- Wave: P6$' "$adr" \
  || fail "$adr missing 'Wave: P6' header"
grep -qE '^- Date: [0-9]{4}-[0-9]{2}-[0-9]{2}$' "$adr" \
  || fail "$adr missing ISO 'Date:' header"

# Required section headings (operator-facing structure).
for section in '^## Context$' '^## Decision$' '^## Consequences$'; do
  grep -qE "$section" "$adr" \
    || fail "$adr missing required section matching $section"
done

# Cross-reference from AGENTS.md (docs-5: ADR must be discoverable
# from the agent-operating manual).
grep -qE '0015-daemon-only-clean-break\.md' "$agents" \
  || fail "AGENTS.md does not cross-reference 0015-daemon-only-clean-break.md"

# Cross-reference from the ADR index table.
grep -qE '0015-daemon-only-clean-break\.md' "$adr_index" \
  || fail "docs/adr/README.md index does not list 0015-daemon-only-clean-break.md"

printf 'tests/adr-0015-presence-eval: OK\n'
