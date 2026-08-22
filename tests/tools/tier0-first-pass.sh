#!/usr/bin/env bash
# Fast source-hygiene checks for repository-owned shell and text inputs.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)

find_repo_root() {
  local candidate path
  for candidate in \
    "$HERE" \
    "${D2B_REPO_ROOT:-}" \
    "${TEST_SRCDIR:-}/${TEST_WORKSPACE:-}" \
    "${TEST_SRCDIR:-}"
  do
    [ -n "$candidate" ] && [ -d "$candidate" ] || continue
    path=$(cd "$candidate" && pwd -P) || continue
    while [ "$path" != "/" ]; do
      if [ -f "$path/BUILD.bazel" ] && [ -f "$path/flake.nix" ] && [ -d "$path/tests" ]; then
        printf '%s\n' "$path"
        return 0
      fi
      path=$(dirname "$path")
    done
  done
  return 1
}

log() {
  printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2
}

fail() {
  log "  FAIL: $*"
  exit 1
}

ok() {
  log "  PASS: $*"
}

ROOT=${ROOT:-$(find_repo_root)} || fail "cannot discover repository root"

is_dash_exempt() {
  case "$1" in
    AGENTS.md|tests/AGENTS.md|labs/venus-vulkan-video/AGENTS.md|CLAUDE.md)
      return 0
      ;;
    third_party/agent-skills/ponytail/v4.9.0/skills/*|\
    third_party/agent-skills/caveman/v2.0.0/skills/*|\
    third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/skills/*|\
    third_party/agent-skills/ponytail/v4.9.0/LICENSE|\
    third_party/agent-skills/caveman/v2.0.0/LICENSE|\
    third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/LICENSE)
      return 0
      ;;
  esac
  return 1
}

scan_dashes() {
  local root="$1"
  local -a files=() scan_files=() batch=()
  local file chunk status all_hits start

  if git -C "$root" rev-parse --show-toplevel >/dev/null 2>&1; then
    while IFS= read -r -d '' file; do
      files+=("$file")
    done < <(git -C "$root" ls-files -z --cached --others --exclude-standard)
  else
    while IFS= read -r -d '' file; do
      files+=("${file#"$root"/}")
    done < <(find "$root" -type f -not -path '*/.git/*' -not -path '*/target/*' -print0)
  fi

  [ "${#files[@]}" -gt 0 ] || fail "source-hygiene scan found no files"
  for file in "${files[@]}"; do
    case "$file" in
      .agent-tmp/*|.git/*|bazel-bin/*|bazel-out/*|bazel-testlogs/*|\
      external/*|local-spawn-runner.*|target/*|*.runfiles/*)
        continue
        ;;
    esac
    [ -f "$root/$file" ] || continue
    is_dash_exempt "$file" && continue
    scan_files+=("$root/$file")
  done
  [ "${#scan_files[@]}" -gt 0 ] || fail "source-hygiene scan found no readable files"
  all_hits=
  set +e
  for ((start = 0; start < ${#scan_files[@]}; start += 100)); do
    batch=("${scan_files[@]:start:100}")
    chunk=$(grep -nI -H -E \
      $'\xE2\x80\x90|\xE2\x80\x91|\xE2\x80\x92|\xE2\x80\x93|\xE2\x80\x94|\xE2\x80\x95|\xE2\x88\x92|\xEF\xB9\x98|\xEF\xBC\x8D' \
      -- "${batch[@]}")
    status=$?
    if [ "$status" -eq 0 ]; then
      all_hits+="${all_hits:+$'\n'}$chunk"
    elif [ "$status" -gt 1 ]; then
      set -e
      fail "source-hygiene scan could not read source inputs"
    fi
  done
  set -e
  if [ -n "$all_hits" ]; then
    printf '%s\n' "$all_hits" >&2
    fail "non-ASCII dash found in source inputs"
  fi
  ok "ASCII dash policy passed for ${#scan_files[@]} files"
}

mapfile -t shell_files < <(
  find "$ROOT/tests" "$ROOT/scripts" "$ROOT/harness/ubuntu" \
    -type f -name '*.sh' 2>/dev/null | sort
)
[ "${#shell_files[@]}" -gt 0 ] || fail "no shell scripts found for source hygiene"

bash -n "${shell_files[@]}"
ok "bash -n on ${#shell_files[@]} shell scripts"

if ! shellcheck_bin=$(command -v shellcheck); then
  fail "shellcheck is required for the source-hygiene gate; enter the declared Nix/Bazel test environment"
fi
"$shellcheck_bin" --severity=warning -x "${shell_files[@]}"
ok "shellcheck --severity=warning on ${#shell_files[@]} shell scripts"

scan_dashes "$ROOT"
ok "source-hygiene gate complete"
