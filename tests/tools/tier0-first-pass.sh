#!/usr/bin/env bash
# tests/tools/tier0-first-pass.sh - sub-60s first-pass PR gate.
#
# Pure host-local checks only:
#   * bash -n on tracked shell scripts under tests/, scripts/, harness/ubuntu/
#   * shellcheck --severity=warning on the same scripts when available
#   * repository-wide ban on every non-ASCII dash codepoint
#   * process-marker ban on shipped and operator-facing artifacts
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

# A process marker is a delimited wave (`W3`, `W4-fu`, `W1fu3`), phase
# (`P6`, `P2.3`, `ph6`), follow-up (`fu3`), contextual finding/revision
# (`finding H20`, `revision R5`), or parenthesized reviewer finding
# (`(rust-1)`). Alphanumeric and underscore boundaries are deliberately
# excluded: this does not match W3C, SHA-like text, v3, H264, W3_ROWS, or
# w4Fu. The last is part of the functional defaultSwitchReadiness contract.
# A hyphenated marker is accepted only after an identifier character, so
# v1.1-P2 is caught while the legitimate command option `-W2` is not.
# Lowercase wave tags are recognized only in path-shaped filenames, such as
# a lowercase wave prefix followed by a distro name; this avoids treating
# ordinary prose tokens as process tags.
PROCESS_MARKER_RE='(^|[^[:alnum:]_-])W[0-9]+((fu|a)[0-9]*|-(fu|followup)([0-9]+)?)?([^[:alnum:]_]|$)|[[:alnum:]_]-W[0-9]+((fu|a)[0-9]*|-(fu|followup)([0-9]+)?)?([^[:alnum:]_]|$)|(^|[^[:alnum:]_-])P[0-9]+([.][0-9]+)?([^[:alnum:]_]|$)|[[:alnum:]_]-P[0-9]+([.][0-9]+)?([^[:alnum:]_]|$)|(^|[^[:alnum:]_])(ph|fu)[0-9]+([^[:alnum:]_]|$)|(^|[^[:alnum:]_])(finding|recommendation|review|panel|round|revision)[[:space:]#:_-]+[CHMLR][0-9]+([^[:alnum:]_]|$)|[(][[:space:]]*(software|test|nixos|networking|security|rust|product|docs|observability|kernel)-[0-9]+[[:space:]]*[)]'
PROCESS_MARKER_FILENAME_RE='(^|[-_.])(W|w|P)[0-9]+((fu|a)[0-9]*|-(fu|followup)([0-9]+)?)?([-_.]|$)'

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

# Fail closed on process markers in artifacts governed by AGENTS.md.
#
# Path classification is the allow-list. Historical/process-bearing paths
# (AGENTS.md, docs/adr/**, docs/specs/**, changelog.d/**) never enter the
# governed set. Shipped prose and CLI goldens are scanned in full. Source trees
# use comment context plus the CLI crate's string context; workflow and
# CHANGELOG files use workflow/job/step-name and released-section contexts. A
# new path exemption therefore requires changing this code, not adding magic
# prose that the scanner silently accepts.
#
# Enumeration is intentionally identical to scan_dashes: git supplies every
# tracked and non-ignored untracked file, while an isolated fixture uses a
# pruned find. NUL-safe collection preserves unusual names. Only grep/awk's
# clean statuses are accepted; unreadable, vanished, or otherwise unclassifiable
# governed files fail instead of being skipped.
scan_process_markers() {
  local root="$1"
  local -a files=() full_files=() source_files=() filename_files=()
  local -a workflow_files=() changelog_files=()
  local f hits context_hits toplevel enum_status grep_status awk_status
  local is_repo_root=0 filename_hits=

  root=$(cd "$root" && pwd -P)
  toplevel=$(git -C "$root" rev-parse --show-toplevel 2>/dev/null || true)

  local lastpipe_was_set=1
  shopt -q lastpipe || lastpipe_was_set=0
  shopt -s lastpipe
  set +e
  if [ -n "$toplevel" ] && [ "$(cd "$toplevel" && pwd -P)" = "$root" ]; then
    is_repo_root=1
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
    || fail "process-marker scan could not enumerate files under $root (enumerator exited $enum_status)"
  [ "${#files[@]}" -gt 0 ] || fail "process-marker scan found no files under $root"

  for f in "${files[@]}"; do
    if [ "$is_repo_root" -eq 0 ]; then
      full_files+=("$f")
      filename_files+=("$f")
      continue
    fi
    case "$f" in
      AGENTS.md|docs/adr/*|docs/specs/*|changelog.d/*)
        ;;
      README.md|SECURITY.md|docs/reference/*|docs/how-to/*|docs/explanation/*|examples/*/README*)
        full_files+=("$f")
        filename_files+=("$f")
        ;;
      tests/golden/cli-output/*)
        full_files+=("$f")
        filename_files+=("$f")
        ;;
      tests/golden/l3-matrix/*)
        filename_files+=("$f")
        ;;
      tests/fixtures/gen-w3-cli-goldens.py)
        full_files+=("$f")
        filename_files+=("$f")
        ;;
      nixos-modules/*|pkgs/*|packages/*)
        source_files+=("$f")
        ;;
      .github/workflows/*)
        workflow_files+=("$f")
        filename_files+=("$f")
        ;;
      CHANGELOG.md)
        changelog_files+=("$f")
        ;;
    esac
  done

  [ "$is_repo_root" -eq 0 ] || [ "${#full_files[@]}" -gt 0 ] \
    || fail "process-marker scan could not classify any governed files under $root"

  for f in "${full_files[@]}" "${source_files[@]}" "${workflow_files[@]}" "${filename_files[@]}"; do
    [ -r "$root/$f" ] \
      || fail "process-marker scan cannot read governed file $f"
  done
  for f in "${filename_files[@]}"; do
    if [[ "$(basename "$f")" =~ $PROCESS_MARKER_FILENAME_RE ]]; then
      filename_hits+="${filename_hits:+$'\n'}$f: filename contains a process marker"
    fi
  done

  if [ "${#full_files[@]}" -gt 0 ]; then
    if hits=$(cd "$root" && grep -nHE -e "$PROCESS_MARKER_RE" -- "${full_files[@]}" 2>&1); then
      grep_status=0
    else
      grep_status=$?
    fi
    if [ "$grep_status" -gt 1 ]; then
      [ -n "$hits" ] && printf '%s\n' "$hits" >&2
      fail "process-marker scan aborted: grep exited $grep_status (unreadable/vanished file or bad pattern) under $root"
    fi
  else
    hits=
    grep_status=1
  fi

  if [ "${#source_files[@]}" -gt 0 ]; then
    set +e
    context_hits=$(
      cd "$root" && awk -v marker="$PROCESS_MARKER_RE" '
        {
          marker_at = match($0, marker)
          if (!marker_at) {
            next
          }
          prefix = substr($0, 1, marker_at)
          comment_at = match(prefix, /(^|[[:space:]])(\/\/[/!]?|#|\/\*|\*)/)
          cli_string = FILENAME ~ /^packages\/d2b\/src\// &&
            prefix ~ /["]/
          nix_string = (FILENAME ~ /^nixos-modules\// ||
                        FILENAME ~ /^pkgs\//) &&
            prefix ~ /["]/
          if (comment_at || cli_string || nix_string) {
            printf "%s:%d:%s\n", FILENAME, FNR, $0
          }
        }
      ' "${source_files[@]}"
    )
    awk_status=$?
    set -e
    [ "$awk_status" -eq 0 ] \
      || fail "process-marker source-context scan aborted (awk exited $awk_status)"
    [ -z "$context_hits" ] || hits+="${hits:+$'\n'}$context_hits"
  fi

  for f in "${workflow_files[@]}"; do
    set +e
    context_hits=$(
      cd "$root" && awk -v path="$f" -v marker="$PROCESS_MARKER_RE" '
        $0 ~ marker &&
          ($0 ~ /^[[:space:]]*(-[[:space:]]+)?name[[:space:]]*:/ ||
           $0 ~ /^  [[:alnum:]_.-]+[[:space:]]*:/) {
          printf "%s:%d:%s\n", path, NR, $0
        }
      ' "$f"
    )
    awk_status=$?
    set -e
    [ "$awk_status" -eq 0 ] \
      || fail "process-marker workflow scan aborted for $f (awk exited $awk_status)"
    [ -z "$context_hits" ] || hits+="${hits:+$'\n'}$context_hits"
  done

  for f in "${changelog_files[@]}"; do
    [ -r "$root/$f" ] \
      || fail "process-marker scan cannot read governed file $f"
    set +e
    context_hits=$(
      cd "$root" && awk -v path="$f" -v marker="$PROCESS_MARKER_RE" '
        /^## \[Unreleased\]/ { released = 0; next }
        /^## \[[^]]+\]/ { released = 1; next }
        released && $0 ~ marker {
          printf "%s:%d:%s\n", path, NR, $0
        }
      ' "$f"
    )
    awk_status=$?
    set -e
    [ "$awk_status" -eq 0 ] \
      || fail "process-marker CHANGELOG scan aborted for $f (awk exited $awk_status)"
    [ -z "$context_hits" ] || hits+="${hits:+$'\n'}$context_hits"
  done

  if [ -n "$filename_hits" ]; then
    hits+="${hits:+$'\n'}$filename_hits"
  fi
  if [ -n "$hits" ] || [ "$grep_status" -eq 0 ]; then
    [ -n "$hits" ] && printf '%s\n' "$hits" >&2
    fail "internal process marker matched a shipped or operator-facing artifact under $root (see output above)"
  fi
  ok "no process markers in shipped or operator-facing artifacts"
}

# Exposed so the gate's own test can drive the scan over a fixture tree.
if [ "${1:-}" = "--scan-dashes" ]; then
  scan_dashes "${2:-$ROOT}"
  exit 0
fi
if [ "${1:-}" = "--scan-process-markers" ]; then
  scan_process_markers "${2:-$ROOT}"
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
scan_process_markers "$ROOT"

ok "tier0 fast gate complete"
