#!/usr/bin/env bash
# P7 ph7-p7-changelog-cut gate. Asserts CHANGELOG.md has cut the v1.0.0
# release: a "## 1.0.0" header is present, it carries a "Breaking
# changes (summary)" block enumerating the P0-P6 breaking changes with
# cross-references to ADR 0015 and the per-phase deliverable docs, and
# an empty "## Unreleased" section sits above it ready to accumulate
# post-1.0.0 entries. Layer-1, eval-only.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

changelog="$ROOT/CHANGELOG.md"

fail() {
  printf 'tests/changelog-v1-cut-eval: %s\n' "$*" >&2
  exit 1
}

[ -f "$changelog" ] || fail "missing $changelog"

unreleased_line=$(grep -nE '^## Unreleased$' "$changelog" | head -1 | cut -d: -f1 || true)
[ -n "$unreleased_line" ] \
  || fail "no '## Unreleased' header found"

v1_line=$(grep -nE '^## 1\.0\.0( |$)' "$changelog" | head -1 | cut -d: -f1 || true)
[ -n "$v1_line" ] \
  || fail "no '## 1.0.0' header found"

if [ "$unreleased_line" -ge "$v1_line" ]; then
  fail "'## Unreleased' (line $unreleased_line) must appear ABOVE '## 1.0.0' (line $v1_line)"
fi

# v1.0.0 header carries the cut date (YYYY-MM-DD after an em dash or
# hyphen). Accept either em-dash or ASCII hyphen between version and
# date; reject bare "## 1.0.0".
v1_header=$(sed -n "${v1_line}p" "$changelog")
if ! grep -qE '^## 1\.0\.0[[:space:]]+[—-][[:space:]]+[0-9]{4}-[0-9]{2}-[0-9]{2}$' <<< "$v1_header"; then
  fail "'## 1.0.0' header missing 'YYYY-MM-DD' cut date: $v1_header"
fi

# "## Unreleased" must be empty of release content: between the
# Unreleased header and the v1.0.0 header, only blank lines and HTML
# comments are permitted. (No "### " entries, no bullet lines.)
between_start=$((unreleased_line + 1))
between_end=$((v1_line - 1))
if [ "$between_end" -ge "$between_start" ]; then
  body=$(sed -n "${between_start},${between_end}p" "$changelog")
  if grep -qE '^### ' <<< "$body"; then
    fail "'## Unreleased' section is not empty: contains '### ' entry between lines $between_start-$between_end"
  fi
  if grep -qE '^[*-] ' <<< "$body"; then
    fail "'## Unreleased' section is not empty: contains bullet entry between lines $between_start-$between_end"
  fi
fi

# Extract the v1.0.0 section body (from its header to the next "## "
# header) and assert the summary block + cross-refs live inside it.
v1_section=$(awk -v start="$v1_line" '
  NR == start { capture = 1; print; next }
  capture && /^## / { exit }
  capture { print }
' "$changelog")

grep -qE '^### Breaking changes \(summary\)$' <<< "$v1_section" \
  || fail "'## 1.0.0' section missing '### Breaking changes (summary)' block"

# Cross-reference to ADR 0015 in the summary block (binding decision).
grep -qE '0015-daemon-only-clean-break\.md' <<< "$v1_section" \
  || fail "'## 1.0.0' section does not cross-reference ADR 0015"

# Enumerate each P0-P6 breaking change called out in the plan.
required_keywords=(
  'manifestVersion'   # P2 manifest v2 -> v3
  'bash CLI'          # P4/P6 bash CLI removed
  'per-VM systemd'    # P6 per-VM systemd templates retired
  'Host singletons'   # P6 host singletons retired
  'Polkit'            # P6 polkit per-VM allowlists removed
  'W14c'              # P4 W14c bash fallback removed
  'W18'               # P5 W18 default flip
)
for kw in "${required_keywords[@]}"; do
  grep -qF "$kw" <<< "$v1_section" \
    || fail "'### Breaking changes (summary)' missing required keyword: $kw"
done

# Cross-reference to at least one per-phase deliverable doc in the
# summary block (privileges + manifest-schema + cli-contract +
# default-switch + daemon-lifecycle are the load-bearing docs).
phase_docs=(
  'docs/reference/privileges.md'
  'docs/reference/manifest-schema.md'
  'docs/reference/cli-contract.md'
  'docs/explanation/default-switch-and-deprecation.md'
  'docs/explanation/daemon-lifecycle.md'
)
hits=0
for doc in "${phase_docs[@]}"; do
  if grep -qF "$doc" <<< "$v1_section"; then
    hits=$((hits + 1))
  fi
done
if [ "$hits" -lt 3 ]; then
  fail "'### Breaking changes (summary)' cross-references only $hits/${#phase_docs[@]} per-phase deliverable docs (need >= 3)"
fi

printf 'tests/changelog-v1-cut-eval: OK\n'
