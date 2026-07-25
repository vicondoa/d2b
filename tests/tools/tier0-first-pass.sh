#!/usr/bin/env bash
# tests/tools/tier0-first-pass.sh - sub-60s first-pass PR gate.
#
# Pure host-local checks only:
#   * bash -n on tracked shell scripts under tests/, scripts/, harness/ubuntu/
#   * shellcheck --severity=warning on the same scripts when available
#   * repository-wide em-dash (U+2014) ban
#
# Intentionally excludes nix eval, cargo fmt/clippy/test, and derivation
# materialization; those stay in tests/static-fast.sh and tests/static.sh.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}

# U+2014 EM DASH, spelled as a shell escape rather than as the literal
# character. Two reasons, both load-bearing: the gate below bans that character
# repository-wide and would otherwise flag its own source, and a future editor
# cannot "helpfully" retype the pattern as the character it is looking for.
EM_DASH=$'\u2014'

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

# Fail closed on any em-dash under `$1`.
#
# When `$1` is the root of a git work tree the scope is every file git would
# ship plus every untracked file that is not ignored, which excludes .git/,
# target/, result*, .direnv/ and the test scratch directories without
# hand-maintaining a second ignore list. Any other directory (the gate's own
# test fixture) falls back to a pruned find; a fixture nested inside an ignored
# directory is invisible to git ls-files and would otherwise scan nothing. Both
# paths hand grep the whole file list at once rather than looping per file, and
# grep -I drops binaries so the scan cannot choke on one.
scan_em_dash() {
  local root="$1"
  local -a files=()
  local hits toplevel

  root=$(cd "$root" && pwd -P)
  toplevel=$(git -C "$root" rev-parse --show-toplevel 2>/dev/null || true)
  if [ -n "$toplevel" ] && [ "$(cd "$toplevel" && pwd -P)" = "$root" ]; then
    mapfile -d '' files < <(cd "$root" && git ls-files -z --cached --others --exclude-standard)
  else
    mapfile -d '' files < <(cd "$root" && find . -name .git -prune -o -name target -prune -o -type f -print0)
  fi
  [ "${#files[@]}" -gt 0 ] || fail "em-dash scan found no files under $root"

  hits=$(cd "$root" && grep -nHIF -e "$EM_DASH" -- "${files[@]}") || true
  if [ -n "$hits" ]; then
    printf '%s\n' "$hits" >&2
    fail "em-dash (U+2014) is banned repository-wide ($(printf '%s\n' "$hits" | wc -l) line(s) above); use a spaced hyphen ' - ' or restructure the sentence"
  fi
  ok "no em-dash (U+2014) in ${#files[@]} files"
}

# Exposed so the gate's own test can drive the scan over a fixture tree.
if [ "${1:-}" = "--scan-em-dash" ]; then
  scan_em_dash "${2:-$ROOT}"
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

scan_em_dash "$ROOT"

ok "tier0 fast gate complete"
