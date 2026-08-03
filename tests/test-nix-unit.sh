#!/usr/bin/env bash
# tests/test-nix-unit.sh - `make test-nix-unit`: evaluate the complete
# Nix-unit corpus through nix-eval-jobs, or build one CI-selected shard.
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

flake_root=$(git rev-parse --show-toplevel)
flake_ref="git+file://$flake_root"
system=$(nix eval --raw --impure --expr builtins.currentSystem)

if [ -n "${D2B_NIX_UNIT_CHECK:-}" ] \
  && ! [[ "$D2B_NIX_UNIT_CHECK" =~ ^[A-Za-z0-9._-]+$ ]]; then
  fail "D2B_NIX_UNIT_CHECK contains an unsafe check name" || true
  exit 2
fi

check_dir=$(d2b_mktemp ".d2b-nix-unit-checks.XXXXXX")
check_list="$check_dir/checks"
if ! nix eval --raw "${flake_ref}#checks.$system" --apply '
    cs:
      builtins.concatStringsSep "\n"
        (builtins.filter
          (name: name == "nix-unit" || builtins.substring 0 9 name == "nix-unit-")
          (builtins.sort builtins.lessThan (builtins.attrNames cs)))
  ' >"$check_list"; then
  fail "nix-unit corpus ($system): check discovery failed" || true
  exit 1
fi
mapfile -t checks <"$check_list"

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

# `D2B_NIX_UNIT_JOBS` remains a compatibility alias for the established CI
# selector while local evaluation uses nix-eval-jobs' own worker control.
workers=${D2B_NIX_EVAL_JOBS_WORKERS:-${D2B_NIX_UNIT_JOBS:-4}}
case "$workers" in
  1|2|3|4) ;;
  *)
    fail "nix-eval-jobs workers must be an integer from 1 through 4" || true
    exit 2
    ;;
esac
memory_mb=${D2B_NIX_EVAL_JOBS_MEMORY_MB:-${D2B_NIX_UNIT_MEMORY_MB:-4096}}
if ! [[ "$memory_mb" =~ ^[0-9]+$ ]] \
  || [ "$memory_mb" -lt 512 ] \
  || [ "$memory_mb" -gt 8192 ]; then
  fail "nix-eval-jobs per-worker memory must be between 512 and 8192 MiB" || true
  exit 2
fi

if [ -n "${D2B_NIX_UNIT_CHECK:-}" ]; then
  check="$D2B_NIX_UNIT_CHECK"
  log "--> nix build --no-link ${flake_ref}#checks.${system}.${check}"
  if nix build --no-link --print-out-paths \
    "${flake_ref}#checks.${system}.${check}"; then
    ok "nix-unit check $check ($system)"
  else
    fail "nix-unit check $check ($system) failed" || true
    exit 1
  fi
  log "test-nix-unit OK (selected $check; duration: $((SECONDS - suite_started))s)"
  exit 0
fi

if ! command -v nix-eval-jobs >/dev/null 2>&1; then
  fail "nix-eval-jobs is required for local corpus evaluation; acquire it explicitly with 'nix shell nixpkgs#nix-eval-jobs'" || true
  exit 2
fi
if ! command -v jq >/dev/null 2>&1; then
  fail "jq is required to report every nix-eval-jobs attribute result" || true
  exit 2
fi

# nix-eval-jobs owns the evaluator worker pool. It instantiates only the
# dedicated per-case derivations; this path never invokes nix build.
result_dir=$(d2b_mktemp ".d2b-nix-eval-jobs.XXXXXX")
result_file="$result_dir/results.jsonl"
log "--> nix-eval-jobs --flake ${flake_ref}#nixUnitJobs.${system} --workers $workers --max-memory-size $memory_mb"
if nix-eval-jobs \
  --flake "${flake_ref}#nixUnitJobs.${system}" \
  --workers "$workers" \
  --max-memory-size "$memory_mb" \
  --show-trace >"$result_file"; then
  tool_status=0
else
  tool_status=$?
fi

if [ ! -s "$result_file" ] || ! jq -s -e '
  length > 0
  and all(.[]; type == "object"
    and ((.attr? != null) or (.attrPath? != null)))
' "$result_file" >/dev/null; then
  cat "$result_file" >&2 || true
  fail "nix-eval-jobs returned no valid JSON-lines attribute results" || true
  exit 1
fi

cat "$result_file"
mapfile -t failures < <(
  jq -r '
    select(type == "object" and (.error? != null))
    | (.attr // ((.attrPath // []) | join(".")))
      + ": " + (.error | tostring)
  ' "$result_file"
)
result_count=$(jq -s 'length' "$result_file")
integrity_count=$(jq -s '
  [ .[]
    | select(((.attrPath // []) | join(".")) == "__nix_unit_integrity")
  ] | length
' "$result_file")

if [ "$integrity_count" -ne 1 ]; then
  log "  FAIL: nix-unit integrity attribute was not evaluated exactly once"
fi
for failure in "${failures[@]:-}"; do
  log "  FAIL: nix-unit attribute $failure"
done
if [ "$tool_status" -ne 0 ]; then
  log "  FAIL: nix-eval-jobs exited with status $tool_status"
fi
if [ "${#failures[@]}" -ne 0 ] \
  || [ "$tool_status" -ne 0 ] \
  || [ "$integrity_count" -ne 1 ]; then
  fail "nix-unit corpus ($system): ${#failures[@]} attribute failure(s) across $result_count results" || true
  exit 1
fi

log "test-nix-unit OK ($result_count attributes, $workers workers, ${memory_mb}MiB per worker; duration: $((SECONDS - suite_started))s)"
