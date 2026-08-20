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
suite_started=$SECONDS

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
  $'\xE2\x80\x90' $'\xE2\x80\x91' $'\xE2\x80\x92' $'\xE2\x80\x93' $'\xE2\x80\x94'
  $'\xE2\x80\x95' $'\xE2\x88\x92' $'\xEF\xB9\x98' $'\xEF\xBC\x8D'
)

# These are the only repository-owned paths whose upstream agent payloads may
# retain non-ASCII punctuation. The paths are deliberately enumerated instead
# of admitting a broad vendor or adapter directory.
DASH_EXEMPT_INSTRUCTION_PATHS=(
  AGENTS.md
  tests/AGENTS.md
  labs/venus-vulkan-video/AGENTS.md
  CLAUDE.md
)
DASH_APPROVED_SKILL_ROOTS=(
  third_party/agent-skills/ponytail/v4.9.0/skills
  third_party/agent-skills/caveman/v2.0.0/skills
  third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/skills
)
DASH_EXEMPT_NOTICE_PATHS=(
  third_party/agent-skills/ponytail/v4.9.0/LICENSE
  third_party/agent-skills/caveman/v2.0.0/LICENSE
  third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/LICENSE
)
DASH_APPROVED_ADAPTER_ROOTS=(
  .agents/skills
  .claude/skills
)

# A process marker is a delimited wave (`W3`, `W4-fu`, `W1fu3`), phase
# (`P6`, `P2.3`, `ph6`), follow-up (`fu3`), high finding (`H20`),
# contextual finding/revision (`finding M2`, `revision R5`), or reviewer finding
# (`(rust-1)`). Alphanumeric and underscore boundaries are deliberately
# excluded: this does not match W3C, SHA-like text, v3, H264, W3_ROWS, or
# w4Fu. The last is part of the functional defaultSwitchReadiness contract.
# A hyphenated marker is accepted only after an identifier character, so
# v1.1-P2 is caught while the legitimate command option `-W2` is not.
# Lowercase wave tags are recognized only in path-shaped filenames, such as
# a lowercase wave prefix followed by a distro name; this avoids treating
# ordinary prose tokens as process tags.
PROCESS_MARKER_RE='(^|[^[:alnum:]_-])W[0-9]+((fu|a)[0-9]*|-(fu|followup)([0-9]+)?)?([^[:alnum:]_]|$)|[[:alnum:]_]-W[0-9]+((fu|a)[0-9]*|-(fu|followup)([0-9]+)?)?([^[:alnum:]_]|$)|(^|[^[:alnum:]_-])P[0-9]+([.][0-9]+)?([^[:alnum:]_]|$)|[[:alnum:]_]-P[0-9]+([.][0-9]+)?([^[:alnum:]_]|$)|(^|[^[:alnum:]_])(ph|fu)[0-9]+([^[:alnum:]_]|$)|(^|[^[:alnum:]_])H[0-9]{1,2}([^[:alnum:]_]|$)|(^|[^[:alnum:]_])(finding|recommendation|review|revision)[[:space:]#:_-]+[CHMLR][0-9]+([^[:alnum:]_]|$)|[(][[:space:]]*(software|test|nixos|networking|security|rust|product|docs|observability|kernel)-[0-9]+[[:space:]]*[)]'
PROCESS_MARKER_FILENAME_RE='(^|[-_.])(W|w|P)[0-9]+((fu|a)[0-9]*|-(fu|followup)([0-9]+)?)?([-_.]|$)'

# Shrink-only legacy debt ratchet. The committed pin partitions one frozen path
# universe into active and retired entries. A cleanup moves a path from
# activePaths to retiredPaths; adding a path changes the frozen-universe digest
# and fails both this gate and the independent xtask checker. There is no mutable
# count budget to raise alongside an added exemption.
PROCESS_MARKER_PIN=tests/golden/pinned/process-marker-legacy-paths.json
PROCESS_MARKER_UNIVERSE_SHA256=0f6899e939fd8e0b49f41b56a0221f33552d79348adc2853927d338e610f8f34
LEGACY_PROCESS_MARKER_PATHS=()

load_process_marker_pin() {
  local pin="$ROOT/$PROCESS_MARKER_PIN"
  local active_output retired_output digest path
  local -a retired_paths=() universe=()
  local -A seen=()

  [ -r "$pin" ] || fail "cannot read process-marker pin $PROCESS_MARKER_PIN"
  [ "$(grep -c '^  "schemaVersion": 1,$' "$pin" || true)" -eq 1 ] \
    || fail "process-marker pin has an unsupported or ambiguous schemaVersion"
  [ "$(grep -c '^  "activePaths": \[$' "$pin" || true)" -eq 1 ] \
    || fail "process-marker pin must declare exactly one activePaths array"
  [ "$(grep -c '^  "retiredPaths": \[' "$pin" || true)" -eq 1 ] \
    || fail "process-marker pin must declare exactly one retiredPaths array"

  active_output=$(awk '
    /^  "activePaths": \[$/ { inside = 1; next }
    inside && /^  \],?$/ { found_end = 1; exit }
    inside {
      if ($0 !~ /^    "[^"\\]+",?$/) exit 2
      sub(/^    "/, "")
      sub(/",?$/, "")
      print
    }
    END { if (!inside || !found_end) exit 3 }
  ' "$pin") || fail "cannot parse process-marker pin activePaths"
  [ -n "$active_output" ] || fail "process-marker pin activePaths must not be empty"
  mapfile -t LEGACY_PROCESS_MARKER_PATHS <<< "$active_output"

  retired_output=$(awk '
    /^  "retiredPaths": \[/ {
      inside = 1
      if ($0 ~ /\[\]$/) {
        found_end = 1
        exit
      }
      next
    }
    inside && /^  \],?$/ { found_end = 1; exit }
    inside {
      if ($0 !~ /^    "[^"\\]+",?$/) exit 2
      sub(/^    "/, "")
      sub(/",?$/, "")
      print
    }
    END { if (!inside || !found_end) exit 3 }
  ' "$pin") || fail "cannot parse process-marker pin retiredPaths"
  if [ -n "$retired_output" ]; then
    mapfile -t retired_paths <<< "$retired_output"
  fi

  universe=("${LEGACY_PROCESS_MARKER_PATHS[@]}" "${retired_paths[@]}")
  for path in "${universe[@]}"; do
    [ -n "$path" ] || fail "process-marker pin contains an empty path"
    [ -z "${seen[$path]+present}" ] \
      || fail "process-marker pin contains a duplicate path"
    seen["$path"]=1
  done
  command -v sha256sum >/dev/null 2>&1 \
    || fail "sha256sum is required to verify the process-marker pin"
  digest=$(
    printf '%s\n' "${universe[@]}" | LC_ALL=C sort | sha256sum | awk '{ print $1 }'
  ) || fail "cannot compute process-marker pin universe digest"
  [ "$digest" = "$PROCESS_MARKER_UNIVERSE_SHA256" ] \
    || fail "process-marker pin path universe changed; exemptions may only move from activePaths to retiredPaths"
}

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

dash_canonical_skill_dir() {
  local skill="$1"

  case "$skill" in
    ponytail|ponytail-audit|ponytail-debt|ponytail-gain|ponytail-help|ponytail-review)
      printf '%s/%s\n' "${DASH_APPROVED_SKILL_ROOTS[0]}" "$skill"
      ;;
    caveman)
      printf '%s/%s\n' "${DASH_APPROVED_SKILL_ROOTS[1]}" "$skill"
      ;;
    ce-babysit-pr|ce-brainstorm|ce-code-review|ce-commit-push-pr|ce-debug|ce-doc-review|ce-plan|ce-resolve-pr-feedback|ce-simplify-code|ce-work|ce-worktree)
      printf '%s/%s\n' "${DASH_APPROVED_SKILL_ROOTS[2]}" "$skill"
      ;;
    *)
      return 1
      ;;
  esac
}

dash_symlink_matches() {
  local entry="$1" expected="$2"
  local link_target actual expected_real

  [ -L "$entry" ] || return 1
  link_target=$(readlink -- "$entry" 2>/dev/null) || return 1
  case "$link_target" in
    /*) return 1 ;;
  esac
  actual=$(readlink -f -- "$entry" 2>/dev/null) || return 1
  expected_real=$(readlink -f -- "$expected" 2>/dev/null) || return 1
  [ "$actual" = "$expected_real" ]
}

dash_path_is_exempt() {
  local root="$1" f="$2"
  local adapter path skill_root remainder skill component canonical entry expected

  for path in "${DASH_EXEMPT_INSTRUCTION_PATHS[@]}"; do
    [ "$f" = "$path" ] && return 0
  done
  for path in "${DASH_EXEMPT_NOTICE_PATHS[@]}"; do
    [ "$f" = "$path" ] && return 0
  done

  for skill_root in "${DASH_APPROVED_SKILL_ROOTS[@]}"; do
    case "$f" in
      "$skill_root"/*)
        remainder=${f#"$skill_root"/}
        skill=${remainder%%/*}
        [ "$remainder" != "$skill" ] || continue
        canonical=$(dash_canonical_skill_dir "$skill" 2>/dev/null) || continue
        [ "$canonical" = "$skill_root/$skill" ] && return 0
        ;;
    esac
  done

  for adapter in "${DASH_APPROVED_ADAPTER_ROOTS[@]}"; do
    case "$f" in
      "$adapter"/*)
        remainder=${f#"$adapter"/}
        skill=${remainder%%/*}
        canonical=$(dash_canonical_skill_dir "$skill" 2>/dev/null) || continue
        entry="$root/$f"
        if [ "$remainder" = "$skill" ]; then
          dash_symlink_matches "$entry" "$root/$canonical" && return 0
          continue
        fi
        [ "$adapter" = ".claude/skills" ] || continue
        component=${remainder#"$skill"/}
        case "$component" in
          ""|/*|.|./*|*/.|..|../*|*/..|*/../*)
            continue
            ;;
        esac
        expected="$root/$canonical/$component"
        dash_symlink_matches "$entry" "$expected" && return 0
        ;;
    esac
  done

  return 1
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
  local -a files=() patterns=() scan_files=()
  local dash hits toplevel enum_status grep_status
  local enumerated_count exempt_count=0

  root=$(cd "$root" 2>/dev/null && pwd -P) \
    || fail "dash scan cannot resolve its scan root"
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
      | {
        while IFS= read -r -d '' f; do
          # `git ls-files --cached` includes an unstaged deletion. A removed
          # path is not part of the shipped tree and must not be handed to
          # grep as if it were an unreadable file.
          [ -e "$root/$f" ] || [ -L "$root/$f" ] || continue
          files+=("$f")
        done
      }
  else
    (cd "$root" && find . -name .git -prune -o -name target -prune -o \( -type f -o -type l \) -print0) \
      | { while IFS= read -r -d '' f; do files+=("${f#./}"); done; }
  fi
  enum_status=${PIPESTATUS[0]}
  set -e
  [ "$lastpipe_was_set" -eq 1 ] || shopt -u lastpipe

  [ "$enum_status" -eq 0 ] \
    || fail "dash scan could not enumerate files (enumerator exited $enum_status)"
  [ "${#files[@]}" -gt 0 ] || fail "dash scan found no files in its scan root"

  enumerated_count=${#files[@]}
  for f in "${files[@]}"; do
    if dash_path_is_exempt "$root" "$f"; then
      exempt_count=$((exempt_count + 1))
    else
      scan_files+=("$f")
    fi
  done
  files=("${scan_files[@]}")
  if [ "${#files[@]}" -eq 0 ]; then
    ok "no non-ASCII dash in ${enumerated_count} enumerated files (${exempt_count} exempt; grep skipped)"
    return 0
  fi

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
    fail "dash scan aborted: grep exited $grep_status (unreadable/vanished file or bad pattern)"
  fi
  if [ "$grep_status" -eq 0 ]; then
    printf '%s\n' "$hits" >&2
    fail "only the ASCII hyphen '-' may spell a dash; a banned dash codepoint matched (see grep output above)"
  fi
  ok "no non-ASCII dash in ${enumerated_count} enumerated files (${exempt_count} exempt)"
}

# Fail closed on process markers in artifacts governed by AGENTS.md.
#
# Path classification is the allow-list. Historical/process-bearing paths
# (AGENTS.md, docs/adr/**, docs/specs/**, changelog.d/**) never enter the
# governed set. Shipped prose, proof Markdown, CLI goldens, and every CHANGELOG
# section are scanned in full. Production and proof source trees use comment
# context plus the CLI crate's string context; workflow files use
# workflow/job/step-name contexts. A
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
  local -a new_violation_lines=() stale_legacy_paths=()
  local -A legacy_paths=() violation_paths=()
  local f hit path hits context_hits toplevel enum_status grep_status awk_status
  local is_repo_root=0 filename_hits=

  root=$(cd "$root" 2>/dev/null && pwd -P) \
    || fail "process-marker scan cannot resolve its scan root"
  toplevel=$(git -C "$root" rev-parse --show-toplevel 2>/dev/null || true)

  local lastpipe_was_set=1
  shopt -q lastpipe || lastpipe_was_set=0
  shopt -s lastpipe
  set +e
  if [ -n "$toplevel" ] && [ "$(cd "$toplevel" && pwd -P)" = "$root" ]; then
    is_repo_root=1
    (cd "$root" && git ls-files -z --cached --others --exclude-standard) \
      | {
        while IFS= read -r -d '' f; do
          [ -e "$root/$f" ] || [ -L "$root/$f" ] || continue
          files+=("$f")
        done
      }
  else
    (cd "$root" && find . -name .git -prune -o -name target -prune -o \( -type f -o -type l \) -print0) \
      | { while IFS= read -r -d '' f; do files+=("$f"); done; }
  fi
  enum_status=${PIPESTATUS[0]}
  set -e
  [ "$lastpipe_was_set" -eq 1 ] || shopt -u lastpipe

  [ "$enum_status" -eq 0 ] \
    || fail "process-marker scan could not enumerate files (enumerator exited $enum_status)"
  [ "${#files[@]}" -gt 0 ] || fail "process-marker scan found no files in its scan root"

  if [ "$is_repo_root" -eq 1 ]; then
    load_process_marker_pin
    for f in "${LEGACY_PROCESS_MARKER_PATHS[@]}"; do
      [ -z "${legacy_paths[$f]+present}" ] \
        || fail "duplicate process-marker legacy path: $f"
      legacy_paths["$f"]=1
    done
  fi

  for f in "${files[@]}"; do
    if [ "$is_repo_root" -eq 0 ]; then
      full_files+=("$f")
      filename_files+=("$f")
      continue
    fi
    case "$f" in
      # Process-methodology docs. These legitimately carry wave/phase/finding
      # markers because they *document* the methodology rather than ship it.
      # docs/contributing/* is listed explicitly: the case statement has no
      # default arm, so an unlisted path would be silently unclassified and
      # exempt by accident rather than by decision.
      AGENTS.md|docs/contributing/*|docs/adr/*|docs/specs/*|changelog.d/*)
        ;;
      README.md|SECURITY.md|STRATEGY.md|docs/reference/*|docs/how-to/*|docs/explanation/*|examples/*/README*)
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
      proofs/*.md|proofs/*.MD)
        full_files+=("$f")
        filename_files+=("$f")
        ;;
      proofs/*)
        source_files+=("$f")
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
    || fail "process-marker scan could not classify any governed files"

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
      fail "process-marker scan aborted: grep exited $grep_status (unreadable/vanished file or bad pattern)"
    fi
  else
    hits=
    grep_status=1
  fi

  if [ "${#source_files[@]}" -gt 0 ]; then
    set +e
    context_hits=$(
      cd "$root" && awk -v marker="$PROCESS_MARKER_RE" '
        # W0 through W8 are the closed, validated delivery-wave namespace when
        # they occur as exact tokens inside the delivery implementation. Strip
        # only that token shape before applying the process-marker matcher.
        # Suffixed forms such as W0-prep and W4-fu remain visible to the gate.
        function strip_delivery_wave_ids(text, i, previous, following) {
          if (FILENAME !~ /(^|\/)packages\/xtask\/src\/delivery\// ||
              text !~ /(`W[0-8]`|"W[0-8]"|\/W[0-8]\/|ADR046-W[0-8])/) {
            return text
          }
          for (i = 1; i < length(text); i++) {
            if (substr(text, i, 1) != "W" ||
                substr(text, i + 1, 1) !~ /^[0-8]$/) {
              continue
            }
            previous = i == 1 ? "" : substr(text, i - 1, 1)
            following = i + 2 > length(text) ? "" : substr(text, i + 2, 1)
            if ((previous == "" || previous !~ /[[:alnum:]_]/) &&
                (following == "" || following !~ /[[:alnum:]_-]/)) {
              text = substr(text, 1, i - 1) "X" substr(text, i + 1)
            }
          }
          return text
        }
        {
          candidate = strip_delivery_wave_ids($0)
          marker_at = match(candidate, marker)
          if (!marker_at) {
            next
          }
          prefix = substr(candidate, 1, marker_at)
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
        $0 ~ marker {
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
  if [ "$grep_status" -eq 0 ] && [ -z "$hits" ]; then
    fail "process-marker scan matched but produced no classifiable diagnostic"
  fi

  while IFS= read -r hit; do
    [ -n "$hit" ] || continue
    path=${hit%%:*}
    [ "$path" != "$hit" ] \
      || fail "process-marker scan produced an unclassifiable diagnostic: $hit"
    violation_paths["$path"]=1
    if [ -z "${legacy_paths[$path]+present}" ]; then
      new_violation_lines+=("$hit")
    fi
  done <<< "$hits"

  if [ "$is_repo_root" -eq 1 ]; then
    for f in "${LEGACY_PROCESS_MARKER_PATHS[@]}"; do
      if [ -z "${violation_paths[$f]+present}" ]; then
        stale_legacy_paths+=("$f")
      fi
    done
  fi

  if [ "${#new_violation_lines[@]}" -gt 0 ]; then
    printf '%s\n' "${new_violation_lines[@]}" >&2
    fail \
      "new process-marker violation outside the legacy path allow-list; remove the marker from the listed files in this change -" \
      "the frozen pin forbids adding an exemption"
  fi
  if [ "${#stale_legacy_paths[@]}" -gt 0 ]; then
    for f in "${stale_legacy_paths[@]}"; do
      log "  STALE: $f"
    done
    fail \
      "legacy process-marker paths no longer violate; move every STALE entry above from activePaths to retiredPaths in" \
      "$PROCESS_MARKER_PIN; never delete or reactivate a retired entry"
  fi
  if [ "$is_repo_root" -eq 1 ]; then
    ok "process-marker ratchet clean; ${#LEGACY_PROCESS_MARKER_PATHS[@]} legacy paths remain"
  else
    ok "no process markers in shipped or operator-facing artifacts"
  fi
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

ok "tier0 fast gate complete (duration: $((SECONDS - suite_started))s)"
