#!/usr/bin/env bash
# Changelog-cut hygiene gate. Asserts CHANGELOG.md keeps an empty
# "## [Unreleased]" block above the latest dated release, and that the
# historical "## [1.0.0]" daemon-only release section still enumerates
# its breaking changes with a cross-reference to ADR 0015 and the
# v0 -> v1 migration guide. Layer-1, eval-only.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

changelog="$ROOT/CHANGELOG.md"

fail() {
  printf 'tests/changelog-v1-cut-eval: %s\n' "$*" >&2
  exit 1
}

[ -f "$changelog" ] || fail "missing $changelog"

unreleased_line=$(grep -nE '^## \[Unreleased\]$' "$changelog" | head -1 | cut -d: -f1 || true)
[ -n "$unreleased_line" ] \
  || fail "no '## [Unreleased]' header found"

# The latest release is the first "## [X.Y.Z] - DATE" header below
# Unreleased. It must carry a YYYY-MM-DD cut date.
latest_line=$(awk -v u="$unreleased_line" '
  NR > u && /^## \[[0-9]/ { print NR; exit }
' "$changelog")
[ -n "$latest_line" ] \
  || fail "no '## [X.Y.Z]' release header found below '## [Unreleased]'"

latest_header=$(sed -n "${latest_line}p" "$changelog")
if ! grep -qE '^## \[[0-9]+\.[0-9]+(\.[0-9]+)?\][[:space:]]+[—-][[:space:]]+[0-9]{4}-[0-9]{2}-[0-9]{2}$' <<< "$latest_header"; then
  fail "latest release header missing 'YYYY-MM-DD' cut date: $latest_header"
fi

# "## [Unreleased]" must be empty of release content: between it and
# the latest release header, only blank lines / HTML comments are
# permitted (no "### " entries, no bullet lines).
between_start=$((unreleased_line + 1))
between_end=$((latest_line - 1))
if [ "$between_end" -ge "$between_start" ]; then
  body=$(sed -n "${between_start},${between_end}p" "$changelog")
  if grep -qE '^### ' <<< "$body"; then
    fail "'## [Unreleased]' section is not empty: contains '### ' entry"
  fi
  if grep -qE '^[*-] ' <<< "$body"; then
    fail "'## [Unreleased]' section is not empty: contains bullet entry"
  fi
fi

# The historical "## [1.0.0]" daemon-only release section must still
# enumerate its breaking changes and cross-reference ADR 0015.
v1_line=$(grep -nE '^## \[1\.0\.0\]( |$)' "$changelog" | head -1 | cut -d: -f1 || true)
[ -n "$v1_line" ] || fail "no '## [1.0.0]' header found"

v1_section=$(awk -v start="$v1_line" '
  NR == start { capture = 1; print; next }
  capture && /^## / { exit }
  capture { print }
' "$changelog")

grep -qE '^### .*[Bb]reaking' <<< "$v1_section" \
  || fail "'## [1.0.0]' section missing a '(breaking)' group"

# Cross-reference to ADR 0015 in the section (binding decision).
grep -qE '0015-daemon-only-clean-break\.md' <<< "$v1_section" \
  || fail "'## [1.0.0]' section does not cross-reference ADR 0015"

# Enumerate each breaking change called out for the daemon-only cut.
required_keywords=(
  'manifestVersion'
  'bash CLI'
  'per-VM systemd'
  'Host singletons'
  'Polkit'
)
for kw in "${required_keywords[@]}"; do
  grep -qF "$kw" <<< "$v1_section" \
    || fail "'## [1.0.0]' section missing required keyword: $kw"
done

# Cross-reference the operator-facing v0 -> v1 migration guide.
grep -qF 'docs/how-to/migrate-nixling-v0-to-v1.md' <<< "$v1_section" \
  || fail "'## [1.0.0]' section does not cross-reference the v0->v1 migration guide"

printf 'tests/changelog-v1-cut-eval: OK\n'
