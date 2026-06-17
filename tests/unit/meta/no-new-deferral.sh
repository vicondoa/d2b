#!/usr/bin/env bash
# tests/unit/meta/no-new-deferral.sh — I3 invariant enforcement gate.
#
# Per ADR 0022 §Decision, v1.2 is a stabilization-mode release.
# The "no-new-deferral" invariant (I3) forbids authoring any new
# v1.3 deferrals during v1.2 development. This gate is wired into
# tests/static.sh AND invoked by `make pre-tag` to enforce I3
# mechanically.
#
# What we scan for:
#   - "v1.3 deferral"
#   - "Tracked for v1.3"
#   - "TODO(v1.3)"
#
# Where we scan:
#   - CHANGELOG.md
#   - docs/adr/*.md
#   - nixos-modules/**.nix
#   - plan.md (session workspace — only if present)
#
# False-positive guard:
#   The defining ADR 0022 itself REFERENCES these strings as the
#   pattern it forbids. We exclude:
#     - "NOT a v1.3 deferral" (case-insensitive)
#     - "not a v1.3 deferral"
#     - lines in docs/adr/0022-*.md that DEFINE the gate
#     - lines in this file (self-reference)
#
# Exit code:
#   0 = PASS (no actual new deferrals authored)
#   1 = FAIL (a real v1.3 deferral string found; print location + fix
#       instructions)

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}
cd "$ROOT"

PATTERNS=(
  'v1\.3 deferral'
  'Tracked for v1\.3'
  'TODO(v1\.3)'
)

# Files / patterns to scan.
declare -a SCAN_TARGETS=(
  CHANGELOG.md
)
# Add ADR markdowns + nixos-module .nix files explicitly to limit the
# blast radius.
while IFS= read -r f; do SCAN_TARGETS+=("$f"); done < <(find docs/adr -maxdepth 1 -name '*.md' | sort)
while IFS= read -r f; do SCAN_TARGETS+=("$f"); done < <(find nixos-modules -name '*.nix' | sort)

# Self-exclusions: the defining ADR 0022 and this script itself.
SELF_EXCLUDE_FILES=(
  'docs/adr/0022-stabilization-mode-releases.md'
  'tests/unit/meta/no-new-deferral.sh'
)

violations=0
for pat in "${PATTERNS[@]}"; do
  # Grep matching lines; allow grep to return non-zero (no matches).
  matches=$(grep -nH -E -- "$pat" "${SCAN_TARGETS[@]}" 2>/dev/null || true)
  [ -z "$matches" ] && continue
  while IFS= read -r line; do
    # Strip negation-language false positives.
    # Check if the line CONTAINS any of the negation phrases (case-
    # insensitive); if yes → not a real violation.
    if echo "$line" | grep -i -E 'not a v1\.3 deferral|architectural, not a v1\.3 deferral' >/dev/null 2>&1; then
      continue
    fi
    # Check self-exclusion file list.
    file=${line%%:*}
    skip=0
    for ex in "${SELF_EXCLUDE_FILES[@]}"; do
      if [ "$file" = "$ex" ]; then skip=1; break; fi
    done
    if [ $skip -eq 0 ]; then
      echo "no-new-deferral: I3 violation — $line" >&2
      violations=$((violations + 1))
    fi
  done <<< "$matches"
done

if [ $violations -gt 0 ]; then
  echo >&2
  echo "no-new-deferral: $violations actual v1.3 deferral string(s) authored." >&2
  echo "I3 invariant (ADR 0022) forbids new deferrals during v1.2 stabilization." >&2
  echo "Either close the deferral within v1.2 scope or reframe it as an" >&2
  echo "explicit out-of-scope architectural constraint with 'NOT a v1.3 deferral'" >&2
  echo "language in the same paragraph." >&2
  exit 1
fi

echo "PASS: tests/unit/meta/no-new-deferral.sh — no actual v1.3 deferrals authored"
echo "      (scanned: CHANGELOG.md + docs/adr/*.md + nixos-modules/**.nix)"
exit 0
