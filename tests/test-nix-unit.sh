#!/usr/bin/env bash
# tests/test-nix-unit.sh - `make test-nix-unit`: build the nix-unit corpus checks
# (`flake.checks.<system>.nix-unit*`) for the native system.
#
# This is both the focused target for iterating on the declarative value/throw
# corpus under tests/unit/nix/ and explicit Layer-1 evidence. `test-flake` also
# evaluates these checks, but the dedicated job prevents corpus coverage from
# disappearing behind flake sharding or orchestration drift.

set -euo pipefail
suite_started=$SECONDS

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
D2B_LOG=${D2B_LOG:-/dev/null}
export ROOT D2B_LOG

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"
cd "$ROOT"

system=$(nix eval --raw --impure --expr builtins.currentSystem)
mapfile -t checks < <(
  nix eval --raw ".#checks.$system" --apply '
    cs:
      builtins.concatStringsSep "\n"
        (builtins.filter
          (name: name == "nix-unit" || builtins.substring 0 9 name == "nix-unit-")
          (builtins.sort builtins.lessThan (builtins.attrNames cs)))
  '
)

if [ "${#checks[@]}" -eq 0 ]; then
  fail "nix-unit corpus ($system): no nix-unit* checks found"
  exit 1
fi

if [ -n "${D2B_NIX_UNIT_CHECK:-}" ]; then
  if ! [[ "$D2B_NIX_UNIT_CHECK" =~ ^[A-Za-z0-9._-]+$ ]]; then
    fail "D2B_NIX_UNIT_CHECK contains an unsafe check name" || true
    exit 2
  fi
  selected=0
  for check in "${checks[@]}"; do
    if [ "$check" = "$D2B_NIX_UNIT_CHECK" ]; then
      selected=1
      checks=("$check")
      break
    fi
  done
  if [ "$selected" -ne 1 ]; then
    fail "D2B_NIX_UNIT_CHECK is not a discovered nix-unit check: $D2B_NIX_UNIT_CHECK" || true
    exit 2
  fi
fi

# Two evaluators are the conservative default on development hosts. Four is
# the hard ceiling everywhere: larger local fan-out exhausts ordinary hosts,
# and CI expresses the same ceiling as the shard matrix's max-parallel value.
jobs=${D2B_NIX_UNIT_JOBS:-2}
case "$jobs" in
  1|2|3|4) ;;
  *)
    fail "D2B_NIX_UNIT_JOBS must be an integer from 1 through 4 (got ${jobs@Q})" || true
    exit 2
    ;;
esac

# Every discovered check gets one process, one log, and one wait. The bounded
# scheduler overlaps independent pure-eval shards without changing the corpus
# or allowing a fast failure to cancel slower siblings. Logs are replayed in
# discovery order so parallel output never interleaves and every failing shard
# is reported in one run.
log_dir=$(d2b_mktemp ".d2b-nix-unit-logs.XXXXXX")
declare -a pids=() labels=() logs=()
failures=()
next_to_wait=0
running=0

stop_running_checks() {
  local pid
  for pid in "${pids[@]:-}"; do
    kill "$pid" 2>/dev/null || true
  done
}
add_cleanup stop_running_checks

launch_check() {
  local check="$1" ordinal=${#pids[@]}
  [[ "$check" =~ ^[A-Za-z0-9._-]+$ ]] || {
    fail "nix-unit discovered an unsafe check name: ${check@Q}"
    exit 1
  }
  local output="$log_dir/$ordinal.log"
  log "--> nix build .#checks.$system.$check"
  (
    nix build --no-link --print-out-paths ".#checks.$system.$check"
  ) >"$output" 2>&1 &
  pids+=("$!")
  labels+=("$check")
  logs+=("$output")
  running=$((running + 1))
}

reap_next() {
  local index=$next_to_wait check=${labels[$next_to_wait]}
  local output=${logs[$next_to_wait]}
  if wait "${pids[$next_to_wait]}"; then
    cat "$output"
    ok "nix-unit check $check ($system)"
  else
    log "nix-unit check $check FAILED - captured output follows:"
    cat "$output" >&2 || true
    failures+=("$check")
  fi
  next_to_wait=$((index + 1))
  running=$((running - 1))
}

for check in "${checks[@]}"; do
  while [ "$running" -ge "$jobs" ]; do
    reap_next
  done
  launch_check "$check"
done
while [ "$running" -gt 0 ]; do
  reap_next
done

if [ "${#failures[@]}" -ne 0 ]; then
  fail "nix-unit corpus ($system): ${#failures[@]} check(s) failed: ${failures[*]}"
  exit 1
fi

log "test-nix-unit OK (${#checks[@]} checks, up to $jobs workers; duration: $((SECONDS - suite_started))s)"
