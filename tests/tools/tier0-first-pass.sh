#!/usr/bin/env bash
# tests/tools/tier0-first-pass.sh - sub-60s first-pass PR gate.
#
# Pure host-local checks only:
#   * bash -n on tracked shell scripts under tests/, scripts/, harness/ubuntu/
#   * shellcheck --severity=warning on the same scripts when available
#   * repository-wide ban on every non-ASCII dash codepoint
#
# Intentionally excludes nix eval, cargo fmt/clippy/test, and derivation
# materialization; those stay in tests/static-fast.sh and tests/static.sh.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}

# Every non-ASCII dash codepoint. Only the plain ASCII hyphen may spell a dash
# anywhere in this repository, so the whole class is rejected rather than just
# the characters that happen to appear today; a future paste of any of them
# fails the same way.
#
#   U+2010 hyphen            U+2011 non-breaking hyphen  U+2012 figure dash
#   U+2013 en dash           U+2014 em dash              U+2015 horizontal bar
#   U+2212 minus sign        U+FE58 small em dash        U+FF0D fullwidth hyphen
#
# Each is spelled as a shell escape rather than as the literal character. Two
# reasons, both load-bearing: the scan below would otherwise flag its own
# source, and a future editor cannot "helpfully" retype a pattern as the
# character it is looking for.
DASHES=(
  $'\u2010' $'\u2011' $'\u2012' $'\u2013' $'\u2014'
  $'\u2015' $'\u2212' $'\uFE58' $'\uFF0D'
)

log() {
  printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2
}

ok() {
  log "  PASS: $*"
}

fail() {
  log "  FAIL: $*"
  exit 1
}

# Fail closed on any non-ASCII dash under `$1`.
#
# When `$1` is the root of a git work tree the scope is every file git would
# ship plus every untracked file that is not ignored, which excludes .git/,
# target/, result*, .direnv/ and the test scratch directories without
# hand-maintaining a second ignore list. Any other directory (the gate's own
# test fixture) falls back to a pruned find; a fixture nested inside an ignored
# directory is invisible to git ls-files and would otherwise scan nothing. Both
# paths hand grep the whole file list at once rather than looping per file, and
# grep -I drops binaries so the scan cannot choke on one.
scan_dashes() {
  local root="$1"
  local -a files=() patterns=()
  local dash hits toplevel

  root=$(cd "$root" && pwd -P)
  toplevel=$(git -C "$root" rev-parse --show-toplevel 2>/dev/null || true)
  if [ -n "$toplevel" ] && [ "$(cd "$toplevel" && pwd -P)" = "$root" ]; then
    mapfile -d '' files < <(cd "$root" && git ls-files -z --cached --others --exclude-standard)
  else
    mapfile -d '' files < <(cd "$root" && find . -name .git -prune -o -name target -prune -o -type f -print0)
  fi
  [ "${#files[@]}" -gt 0 ] || fail "dash scan found no files under $root"

  for dash in "${DASHES[@]}"; do
    patterns+=(-e "$dash")
  done

  hits=$(cd "$root" && grep -nHIF "${patterns[@]}" -- "${files[@]}") || true
  if [ -n "$hits" ]; then
    printf '%s\n' "$hits" >&2
    fail "only the ASCII hyphen '-' may spell a dash; a banned dash codepoint appears on $(printf '%s\n' "$hits" | wc -l) line(s) above"
  fi
  ok "no non-ASCII dash in ${#files[@]} files"
}

# Exposed so the gate's own test can drive the scan over a fixture tree.
if [ "${1:-}" = "--scan-dashes" ]; then
  scan_dashes "${2:-$ROOT}"
  exit 0
fi

log "==> tests/tools/tier0-first-pass.sh"
cd "$ROOT"

mapfile -t shell_files < <(find tests scripts harness/ubuntu -type f -name '*.sh' 2>/dev/null | sort)
[ "${#shell_files[@]}" -gt 0 ] || fail "no shell scripts found for tier0 gate"

bash -n "${shell_files[@]}"
ok "bash -n on ${#shell_files[@]} shell scripts"

if command -v shellcheck >/dev/null 2>&1; then
  shellcheck --severity=warning -x "${shell_files[@]}"
  ok "shellcheck --severity=warning on ${#shell_files[@]} shell scripts"
else
  log "  SKIP: shellcheck not installed; syntax-only tier0 pass"
fi

scan_dashes "$ROOT"

ok "tier0 fast gate complete"
