#!/usr/bin/env bash
# tests/adr-index-coverage.sh— ADR index coverage guard.
#
# Asserts SET EQUALITY between:
#   - docs/adr/0NNN-*.md files present on disk
#   - ADR entries linked in docs/adr/README.md (href form: (0NNN-*.md))
#
# On mismatch: prints orphans in both directions and exits 1.
# On pass:     prints "PASS: N ADR files indexed in README.md" and exits 0.
#
# Usage:
#   bash tests/adr-index-coverage.sh
#   ROOT=/path/to/clone bash tests/adr-index-coverage.sh

set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}
ADR_DIR="$ROOT/docs/adr"
ADR_README="$ADR_DIR/README.md"

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

[ -d "$ADR_DIR" ]    || fail "docs/adr/ directory not found under ROOT=$ROOT"
[ -f "$ADR_README" ] || fail "docs/adr/README.md not found"

# --- Enumerate files ---
# Extract 4-digit ADR numbers from filenames: 0NNN-*.md (excludes README.md)
mapfile -t file_nums < <(
  find "$ADR_DIR" -maxdepth 1 -name '0[0-9][0-9][0-9]-*.md' -type f \
    | sed 's|.*/\(0[0-9][0-9][0-9]\)-.*|\1|' \
    | sort -u
)

# --- Parse README ---
# Extract 4-digit ADR numbers from markdown href targets of the form (0NNN-*.md)
mapfile -t readme_nums < <(
  grep -oE '\(0[0-9]{3}-[^)]+\.md\)' "$ADR_README" \
    | grep -oE '0[0-9]{3}' \
    | sort -u
)

[ "${#file_nums[@]}" -gt 0 ]   || fail "no ADR files (0NNN-*.md) found in docs/adr/"
[ "${#readme_nums[@]}" -gt 0 ] || fail "no ADR hrefs found in docs/adr/README.md"

# --- Set equality check ---
files_only=()
readme_only=()

# Files missing from README
for n in "${file_nums[@]}"; do
  found=0
  for r in "${readme_nums[@]}"; do
    [ "$n" = "$r" ] && { found=1; break; }
  done
  [ "$found" -eq 1 ] || files_only+=("$n")
done

# README entries without a matching file
for r in "${readme_nums[@]}"; do
  found=0
  for n in "${file_nums[@]}"; do
    [ "$r" = "$n" ] && { found=1; break; }
  done
  [ "$found" -eq 1 ] || readme_only+=("$r")
done

if [ "${#files_only[@]}" -gt 0 ] || [ "${#readme_only[@]}" -gt 0 ]; then
  if [ "${#files_only[@]}" -gt 0 ]; then
    printf 'FAIL: ADR files on disk with no README.md row:\n' >&2
    for n in "${files_only[@]}"; do
      printf '  docs/adr/%s-*.md\n' "$n" >&2
    done
  fi
  if [ "${#readme_only[@]}" -gt 0 ]; then
    printf 'FAIL: README.md rows with no matching ADR file:\n' >&2
    for n in "${readme_only[@]}"; do
      printf '  README.md references %s but docs/adr/%s-*.md not found\n' "$n" "$n" >&2
    done
  fi
  exit 1
fi

printf 'PASS: %d ADR files indexed in README.md\n' "${#file_nums[@]}"
