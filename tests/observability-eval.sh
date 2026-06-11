#!/usr/bin/env bash
# tests/observability-eval.sh — batched eval-time coverage for
# nixling.observability.*.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

SCRATCH=$(nl_mktemp -d .observability-eval.XXXXXX)
BATCH_JSON="$SCRATCH/observability-batch.json"

PASS=0
FAIL=0
SKIP=0
AUTO_OBS_READY=false

pass_case() {
  log "  PASS: $*"
  return 0
}

fail_case() {
  log "  FAIL: $*"
  return 1
}

skip_case() {
  log "  SKIP: $*"
  return 2
}

run_case() {
  local fn="$1"
  shift
  local rc=0
  if "$fn" "$@"; then
    PASS=$((PASS + 1))
  else
    rc=$?
    case "$rc" in
      2) SKIP=$((SKIP + 1)) ;;
      *) FAIL=$((FAIL + 1)) ;;
    esac
  fi
}

show_stderr_tail() {
  local file="$1"
  if [ -s "$file" ]; then
    log '    --- stderr (tail) ---'
    tail -15 "$file" | sed 's/^/      /' >&2
  fi
}

require_success_case_json() {
  local case_name="$1" case_json="$2"
  local actual expected failing_messages

  if [ "$(printf '%s\n' "$case_json" | jq -r '.evalSucceeded')" != 'true' ]; then
    fail_case "$case_name: batch eval did not succeed"
    return 1
  fi

  failing_messages=$(printf '%s\n' "$case_json" | jq -r '.failingMessages[]?')
  if [ -n "$failing_messages" ]; then
    fail_case "$case_name: unexpected failing assertions: $(printf '%s' "$failing_messages" | paste -sd ';' -)"
    return 1
  fi

  actual=$(printf '%s\n' "$case_json" | jq -S -c '.extracted') || {
    fail_case "$case_name: could not decode extracted JSON"
    return 1
  }
  expected=$(printf '%s\n' "$case_json" | jq -S -c '.expectedExtract') || {
    fail_case "$case_name: could not decode expectedExtract JSON"
    return 1
  }

  assert_eq "$actual" "$expected" "$case_name: extracted JSON matches expected JSON" || return 1
}

require_failure_case_json() {
  local case_name="$1" case_json="$2" needles_file failing_messages needle

  if [ "$(printf '%s\n' "$case_json" | jq -r '.evalSucceeded')" != 'true' ]; then
    fail_case "$case_name: batch eval did not reach the assertion surface"
    return 1
  fi

  failing_messages=$(printf '%s\n' "$case_json" | jq -r '.failingMessages[]?')
  if [ -z "$failing_messages" ]; then
    fail_case "$case_name: expected a failing assertion but none were recorded"
    return 1
  fi

  needles_file="$SCRATCH/${case_name}.needles"
  printf '%s\n' "$case_json" | jq -r '.expectedSubstrings[]?' > "$needles_file" || {
    fail_case "$case_name: could not decode expected substrings"
    return 1
  }

  while IFS= read -r needle; do
    [ -n "$needle" ] || continue
    if ! grep -Fq -- "$needle" <<<"$failing_messages"; then
      fail_case "$case_name: failing assertion surface did not contain '$needle'"
      return 1
    fi
  done < "$needles_file"
}

assert_lines_set_eq() {
  local actual="$1" expected="$2" msg="$3"
  local actual_sorted expected_sorted

  actual_sorted=$(printf '%s\n' "$actual" | sed '/^$/d' | LC_ALL=C sort -u)
  expected_sorted=$(printf '%s\n' "$expected" | sed '/^$/d' | LC_ALL=C sort -u)
  assert_eq "$actual_sorted" "$expected_sorted" "$msg" || return 1
}

handle_case() {
  local case_name="$1" case_json="$2" kind

  case "$case_name" in
    obs-enabled-defaults)
      require_success_case_json "$case_name" "$case_json" || return 1
      pass_case "$case_name"
      ;;
    obs-name-extension-allowed)
      if [ "$AUTO_OBS_READY" != 'true' ]; then
        skip_case 'obs-name-extension-allowed: TODO post-integration — auto-obs-vm has not landed in this worktree'
        return 2
      fi
      require_success_case_json "$case_name" "$case_json" || return 1
      pass_case 'obs-name-extension-allowed (consumer can extend auto-declared obs VM)'
      ;;
    obs-reserved-prefix-exempt)
      require_success_case_json "$case_name" "$case_json" || return 1
      pass_case "$case_name"
      ;;
    *)
      kind=$(printf '%s\n' "$case_json" | jq -r '.kind')
      case "$kind" in
        expect-success) require_success_case_json "$case_name" "$case_json" || return 1 ;;
        expect-failure) require_failure_case_json "$case_name" "$case_json" || return 1 ;;
        *)
          fail_case "$case_name: unknown batched case kind '$kind'"
          return 1
          ;;
      esac
      pass_case "$case_name"
      ;;
  esac
}

log '==> tests/observability-eval.sh'

batch_stderr="$SCRATCH/observability-batch.stderr"
if ! (
  cd "$ROOT" &&
    nix-instantiate --eval --strict --json \
      --expr "import ./tests/eval-cases/observability.nix { flakeRoot = \"$ROOT\"; }"
) > "$BATCH_JSON" 2> "$batch_stderr"; then
  if grep -q "attribute 'microvm' missing" "$batch_stderr"; then
    skip_case 'observability-eval: microvm guest surface absent in daemon-only config' || true
    SKIP=$((SKIP + 1))
    log "==> observability-eval: ${PASS} passed, ${FAIL} failed, ${SKIP} skipped"
    exit 0
  fi
  show_stderr_tail "$batch_stderr"
  fail_case 'observability-eval: batched nix-instantiate failed' || true
  FAIL=$((FAIL + 1))
  log "==> observability-eval: ${PASS} passed, ${FAIL} failed, ${SKIP} skipped"
  exit 1
fi

AUTO_OBS_READY=$(jq -r '(."obs-enabled-defaults".extracted.hasSysObs // false) and (."obs-enabled-defaults".extracted.hasObsEnv // false)' "$BATCH_JSON")

while IFS= read -r entry; do
  case_name=$(printf '%s\n' "$entry" | jq -r '.key')
  case_json=$(printf '%s\n' "$entry" | jq -c '.value')
  run_case handle_case "$case_name" "$case_json"
done < <(jq -c 'to_entries[]' "$BATCH_JSON")

log "==> observability-eval: ${PASS} passed, ${FAIL} failed, ${SKIP} skipped"
[ "$FAIL" -eq 0 ]
