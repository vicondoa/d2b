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
  local dash hits toplevel enum_status grep_status

  root=$(cd "$root" && pwd -P)
  toplevel=$(git -C "$root" rev-parse --show-toplevel 2>/dev/null || true)

  # Enumerate through a pipe (not process substitution) so PIPESTATUS carries
  # the enumerator's exit status; lastpipe keeps the read loop in this shell so
  # the array survives. A NUL-safe read preserves paths with spaces/newlines.
  # A non-zero enumerator status fails closed instead of scanning a short or
  # empty list as if the tree were clean.
  local lastpipe_was_set=1
  shopt -q lastpipe || lastpipe_was_set=0
  shopt -s lastpipe
  set +e
  if [ -n "$toplevel" ] && [ "$(cd "$toplevel" && pwd -P)" = "$root" ]; then
    (cd "$root" && git ls-files -z --cached --others --exclude-standard) \
      | { while IFS= read -r -d '' f; do files+=("$f"); done; }
  else
    (cd "$root" && find . -name .git -prune -o -name target -prune -o -type f -print0) \
      | { while IFS= read -r -d '' f; do files+=("$f"); done; }
  fi
  enum_status=${PIPESTATUS[0]}
  set -e
  [ "$lastpipe_was_set" -eq 1 ] || shopt -u lastpipe

  [ "$enum_status" -eq 0 ] \
    || fail "dash scan could not enumerate files under $root (enumerator exited $enum_status)"
  [ "${#files[@]}" -gt 0 ] || fail "dash scan found no files under $root"

  for dash in "${DASHES[@]}"; do
    patterns+=(-e "$dash")
  done

  # grep exits 0 on a match, 1 on a clean scan, and >1 on an error (an
  # unreadable or vanished file, a bad pattern). Status is authoritative: a
  # status of 0 is a banned-dash hit even when the notice lands on stderr (a
  # `grep -I`-dropped binary match reports "binary file matches" to stderr and
  # still exits 0), so keying on stdout content alone would fail open. stderr is
  # folded in for the diagnostic. Only status 1 is the clean case; anything
  # greater must fail the gate rather than report a pass having scanned nothing.
  # `if hits=$(...)` suspends errexit while capturing the command-substitution
  # status.
  if hits=$(cd "$root" && grep -nHIF "${patterns[@]}" -- "${files[@]}" 2>&1); then
    grep_status=0
  else
    grep_status=$?
  fi
  if [ "$grep_status" -gt 1 ]; then
    [ -n "$hits" ] && printf '%s\n' "$hits" >&2
    fail "dash scan aborted: grep exited $grep_status (unreadable/vanished file or bad pattern) under $root"
  fi
  if [ "$grep_status" -eq 0 ]; then
    printf '%s\n' "$hits" >&2
    fail "only the ASCII hyphen '-' may spell a dash; a banned dash codepoint matched under $root (see grep output above)"
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
  # Not a coverage gap. This is the fast local path only; the authoritative
  # lint gate is `make test-lint`, which provisions the linter through nix
  # when it is off PATH and fails closed when it cannot. Say so, because a
  # bare "SKIP" reads as "the linter never ran anywhere".
  #
  # Note: do not begin a comment line here with the linter's own name, or it
  # is parsed as a directive and the file fails to lint (SC1072/SC1073).
  log "  SKIP: shellcheck not on PATH here; authoritative gate is 'make test-lint'"
fi

scan_dashes "$ROOT"

ok "tier0 fast gate complete"
