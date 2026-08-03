#!/usr/bin/env bash
# tests/test-nix-unit.sh - `make test-nix-unit`: evaluate the complete
# Nix-unit corpus through nix-eval-jobs, or evaluate one selected shard.
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

if [ "${D2B_NIX_UNIT_JOBS+x}" = x ]; then
  printf '%s\n' \
    "D2B_NIX_UNIT_JOBS is retired; unset it and use D2B_NIX_UNIT_WORKERS (1 through 4)." \
    >&2
  exit 2
fi

# The helper owns the secure evidence lifecycle. Enter it before the first Nix
# evaluation or toolchain process. The marker is only for the helper's child
# and is preserved through the one possible dev-shell re-entry.
if [ -n "${D2B_EXECUTION_MANIFEST:-}" ] \
  && [ "${D2B_NIX_UNIT_MANIFEST_LIFECYCLE:-0}" != 1 ]; then
  manifest_commit=$(git -C "$ROOT" rev-parse --verify HEAD 2>/dev/null || printf '%s' unknown)
  exec perl "$ROOT/tests/tools/execution-manifest.pl" run \
    --manifest "$D2B_EXECUTION_MANIFEST" \
    --target test-nix-unit \
    --commit "$manifest_commit" \
    -- env D2B_NIX_UNIT_MANIFEST_LIFECYCLE=1 \
      bash "$ROOT/tests/test-nix-unit.sh" "$@"
fi

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"
cd "$ROOT"

manifest_exit_publication_enabled=1
nix_unit_surface=nix-unit
nix_unit_command_succeeded=0
nix_unit_baseline_leaves=(
  nix-unit
  nix-unit-daemon
  nix-unit-guest
  nix-unit-misc
  nix-unit-network
  nix-unit-runtime
  nix-unit-state
)

publish_manifest_fragment() {
  local leaf="$1" status="$2"
  [ -n "${D2B_EXECUTION_MANIFEST:-}" ] || return 0
  perl "$ROOT/tests/tools/execution-manifest.pl" fragment \
    --manifest "$D2B_EXECUTION_MANIFEST" \
    --leaf "$leaf" \
    --status "$status"
}

nix_unit_exit() {
  local rc=$?
  trap - EXIT
  if [ "$manifest_exit_publication_enabled" -eq 1 ] \
    && [ -n "${D2B_EXECUTION_MANIFEST:-}" ] \
    && [ -n "$nix_unit_surface" ]; then
    if ! publish_manifest_fragment "$nix_unit_surface" failed; then
      if [ "$nix_unit_command_succeeded" -eq 1 ]; then
        printf '%s\n' \
          "test-nix-unit: required execution-manifest fragment publication failed after successful surface '$nix_unit_surface'; evidence is incomplete; retry the target." \
          >&2
      else
        printf '%s\n' \
          "test-nix-unit: failed to record failed Nix-unit surface '$nix_unit_surface' in the execution manifest; preserving the original status." \
          >&2
      fi
    fi
  fi
  run_cleanups || true
  exit "$rc"
}
trap nix_unit_exit EXIT

flake_root=$(git rev-parse --show-toplevel)
flake_ref=$(d2b_flake_ref "$flake_root")
flake_label=d2b
if ! command -v nix >/dev/null 2>&1; then
  fail "nix is required for Nix-unit discovery and the locked toolchain shell" || true
  exit 2
fi
system=$(nix eval --raw --impure --expr builtins.currentSystem)

if [ -n "${D2B_NIX_UNIT_CHECK:-}" ] \
  && ! [[ "$D2B_NIX_UNIT_CHECK" =~ ^[A-Za-z0-9._-]+$ ]]; then
  fail "D2B_NIX_UNIT_CHECK contains an unsafe check name" || true
  exit 2
fi

# Plain local invocations self-provision only the locked Nix-unit tools. An
# existing shell with both commands on PATH runs directly; a failed re-entry
# cannot recurse indefinitely.
if ! command -v nix-eval-jobs >/dev/null 2>&1 \
  || ! command -v jq >/dev/null 2>&1; then
  if [ "${D2B_NIX_UNIT_TOOLCHAIN_REENTRY:-0}" = 1 ]; then
    fail "locked nix-unit shell did not provide nix-eval-jobs and jq" || true
    exit 2
  fi
  export D2B_NIX_UNIT_TOOLCHAIN_REENTRY=1
  exec nix develop --quiet --no-warn-dirty --no-write-lock-file \
    "${flake_ref}#devShells.${system}.nix-unit" \
    --command env D2B_NIX_UNIT_TOOLCHAIN_REENTRY=1 \
      bash "$ROOT/tests/test-nix-unit.sh" "$@"
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

missing_checks=()
for expected_check in "${nix_unit_baseline_leaves[@]}"; do
  found=0
  for check in "${checks[@]}"; do
    if [ "$check" = "$expected_check" ]; then
      found=1
      break
    fi
  done
  [ "$found" -eq 1 ] || missing_checks+=("$expected_check")
done
if [ "${#checks[@]}" -ne "${#nix_unit_baseline_leaves[@]}" ] \
  || [ "${#missing_checks[@]}" -ne 0 ]; then
  fail "nix-unit corpus ($system): discovery must contain exactly the seven baseline checks" || true
  [ "${#missing_checks[@]}" -eq 0 ] \
    || log "  missing discovered checks: ${missing_checks[*]}"
  exit 1
fi

if [ -n "${D2B_NIX_UNIT_CHECK:-}" ]; then
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

# `nix-eval-jobs` owns evaluation concurrency. The request is deliberately
# bounded before it is capped by the host's CPU and available memory.
if [ -n "${D2B_NIX_UNIT_WORKERS:-}" ]; then
  requested_workers=$D2B_NIX_UNIT_WORKERS
elif [ "${GITHUB_ACTIONS:-}" = true ]; then
  requested_workers=1
else
  requested_workers=4
fi
case "$requested_workers" in
  1|2|3|4) ;;
  *)
    fail "D2B_NIX_UNIT_WORKERS must be an integer from 1 through 4" || true
    exit 2
    ;;
esac
if [ -n "${D2B_NIX_UNIT_MEMORY_MB:-}" ]; then
  memory_mb=$D2B_NIX_UNIT_MEMORY_MB
elif [ "${GITHUB_ACTIONS:-}" = true ]; then
  memory_mb=3072
else
  memory_mb=4096
fi
if ! [[ "$memory_mb" =~ ^[0-9]+$ ]] \
  || [ "$memory_mb" -lt 512 ] \
  || [ "$memory_mb" -gt 4096 ]; then
  fail "D2B_NIX_UNIT_MEMORY_MB must be between 512 and 4096 MiB" || true
  exit 2
fi

logical_cpus=$(getconf _NPROCESSORS_ONLN 2>/dev/null || nproc 2>/dev/null || printf '%s' 1)
case "$logical_cpus" in
  ''|*[!0-9]*) logical_cpus=1 ;;
esac
[ "$logical_cpus" -ge 1 ] || logical_cpus=1
cpu_cap=$logical_cpus
[ "$cpu_cap" -le 4 ] || cpu_cap=4

mem_available_kib=$(awk '/^MemAvailable:/ { print $2; exit }' /proc/meminfo 2>/dev/null || true)
case "$mem_available_kib" in
  ''|*[!0-9]*) mem_available_kib=0 ;;
esac
effective_memory_bytes=$((mem_available_kib * 1024))
cgroup_memory_unknown=0
cgroup_cpu_unknown=0
if grep -q '^0::' /proc/self/cgroup 2>/dev/null; then
  cgroup_path=$(sed -n 's/^0::\(.*\)$/\1/p' /proc/self/cgroup | head -1)
  cgroup_dir="/sys/fs/cgroup${cgroup_path}"
  if [ -r "$cgroup_dir/cpu.max" ]; then
    cpu_quota=$(awk '{ print $1; exit }' "$cgroup_dir/cpu.max" 2>/dev/null || true)
    cpu_period=$(awk '{ print $2; exit }' "$cgroup_dir/cpu.max" 2>/dev/null || true)
    if [[ "$cpu_quota" =~ ^[0-9]+$ ]] \
      && [[ "$cpu_period" =~ ^[0-9]+$ ]] \
      && [ "$cpu_period" -gt 0 ]; then
      cgroup_cpu_cap=$((cpu_quota / cpu_period))
      [ "$cgroup_cpu_cap" -ge 1 ] || cgroup_cpu_cap=1
      [ "$cgroup_cpu_cap" -lt "$cpu_cap" ] && cpu_cap=$cgroup_cpu_cap
    elif [ "$cpu_quota" != max ]; then
      cgroup_cpu_unknown=1
    fi
  else
    cgroup_cpu_unknown=1
  fi
  if [ -r "$cgroup_dir/memory.max" ] \
    && [ -r "$cgroup_dir/memory.high" ] \
    && [ -r "$cgroup_dir/memory.current" ] \
    && [ -r "$cgroup_dir/memory.stat" ]; then
    cgroup_limit_max=$(cat "$cgroup_dir/memory.max" 2>/dev/null || true)
    cgroup_limit_high=$(cat "$cgroup_dir/memory.high" 2>/dev/null || true)
    cgroup_limit=
    if [[ "$cgroup_limit_max" =~ ^[0-9]+$ ]] \
      && [[ "$cgroup_limit_high" =~ ^[0-9]+$ ]]; then
      cgroup_limit=$cgroup_limit_max
      [ "$cgroup_limit_high" -lt "$cgroup_limit" ] \
        && cgroup_limit=$cgroup_limit_high
    elif [ "$cgroup_limit_max" = max ] \
      && [ "$cgroup_limit_high" = max ]; then
      cgroup_limit=max
    elif [ "$cgroup_limit_max" = max ] \
      && [[ "$cgroup_limit_high" =~ ^[0-9]+$ ]]; then
      cgroup_limit=$cgroup_limit_high
    elif [[ "$cgroup_limit_max" =~ ^[0-9]+$ ]] \
      && [ "$cgroup_limit_high" = max ]; then
      cgroup_limit=$cgroup_limit_max
    else
      cgroup_memory_unknown=1
    fi
    cgroup_current=$(cat "$cgroup_dir/memory.current" 2>/dev/null || true)
    cgroup_inactive=$(awk '$1 == "inactive_file" { print $2; exit }' "$cgroup_dir/memory.stat" 2>/dev/null || true)
    if [ "$cgroup_memory_unknown" -eq 0 ] \
      && [[ "$cgroup_limit" =~ ^[0-9]+$ ]] \
      && [[ "$cgroup_current" =~ ^[0-9]+$ ]] \
      && [[ "$cgroup_inactive" =~ ^[0-9]+$ ]]; then
      cgroup_usage=$((cgroup_current - cgroup_inactive))
      [ "$cgroup_usage" -ge 0 ] || cgroup_usage=0
      cgroup_allowance=$((cgroup_limit - cgroup_usage))
      [ "$cgroup_allowance" -ge 0 ] || cgroup_allowance=0
      [ "$cgroup_allowance" -lt "$effective_memory_bytes" ] \
        && effective_memory_bytes=$cgroup_allowance
    elif [ "$cgroup_limit" != max ]; then
      cgroup_memory_unknown=1
    fi
  else
    cgroup_memory_unknown=1
  fi
fi
if [ "$cgroup_cpu_unknown" -eq 1 ]; then
  cpu_cap=1
  log "  WARN: Nix-unit cgroup CPU state is unreadable; effective workers capped at 1"
fi
if [ "$cgroup_memory_unknown" -eq 1 ]; then
  memory_cap=1
  log "  WARN: Nix-unit cgroup memory state is unreadable; effective workers capped at 1"
else
  available_mb=$((effective_memory_bytes / 1024 / 1024))
  reserve_mb=3072
  worker_budget_mb=$((memory_mb + 2048))
  if [ "$available_mb" -le "$reserve_mb" ]; then
    memory_cap=1
  else
    memory_cap=$(((available_mb - reserve_mb) / worker_budget_mb))
    [ "$memory_cap" -ge 1 ] || memory_cap=1
  fi
fi
workers=$requested_workers
[ "$workers" -le "$cpu_cap" ] || workers=$cpu_cap
[ "$workers" -le "$memory_cap" ] || workers=$memory_cap
[ "$workers" -ge 1 ] || workers=1
log "  nix-eval-jobs workers: requested $requested_workers, effective $workers (CPU cap $cpu_cap, memory cap $memory_cap, $memory_mb MiB evaluator limit plus 2048 MiB overhead per worker)"

if [ -n "${D2B_NIX_UNIT_CHECK:-}" ]; then
  check="$D2B_NIX_UNIT_CHECK"
  nix_unit_surface="$check"
  log "--> nix eval --raw ${flake_label}#checks.${system}.${check}.drvPath (instantiate-only)"
  if nix eval --raw "${flake_ref}#checks.${system}.${check}.drvPath" >/dev/null; then
    nix_unit_command_succeeded=1
    if ! publish_manifest_fragment "$check" passed; then
      printf '%s\n' \
        "test-nix-unit: required execution-manifest fragment publication failed after successful surface '$check'; evidence is incomplete; retry the target." \
        >&2
      exit 1
    fi
    nix_unit_surface=
    ok "nix-unit check $check ($system)"
  else
    fail "nix-unit check $check ($system) failed" || true
    exit 1
  fi
  log "test-nix-unit OK (selected $check; duration: $((SECONDS - suite_started))s)"
  exit 0
fi

if ! command -v nix-eval-jobs >/dev/null 2>&1; then
  fail "nix-eval-jobs is required for local corpus evaluation; the locked nix-unit shell should provide it" || true
  exit 2
fi
if ! command -v jq >/dev/null 2>&1; then
  fail "jq is required to report every nix-eval-jobs attribute result; the locked nix-unit shell should provide it" || true
  exit 2
fi

# nix-eval-jobs owns the evaluator worker pool. It evaluates one aggregate
# attr per case file, while --no-instantiate never submits them as installables
# to the daemon or realizes their outputs.
nix_unit_surface=nix-unit
result_dir=$(d2b_mktemp ".d2b-nix-eval-jobs.XXXXXX")
result_file="$result_dir/results.jsonl"
tool_stderr="$result_dir/stderr"

sanitize_observable_line() {
  local value="$1" store_hash
  value=${value//"$flake_root"/<repo>}
  if [ -n "${HOME:-}" ]; then
    value=${value//"$HOME"/<home>}
  fi
  while [[ "$value" =~ /nix/store/[a-z0-9]{32} ]]; do
    store_hash=${BASH_REMATCH[0]}
    value=${value//"$store_hash"/<store>}
  done
  sanitized_line=$value
}

sanitize_stderr_file() {
  local source="$1" line
  while IFS= read -r line || [ -n "$line" ]; do
    sanitize_observable_line "$line"
    printf '%s\n' "$sanitized_line" >&2
  done <"$source"
}

emit_sanitized_tool_stderr() {
  sanitize_stderr_file "$tool_stderr"
}

inventory_file="$result_dir/inventory.json"
inventory_stderr="$result_dir/inventory.stderr"
if ! nix eval \
  --impure \
  --quiet \
  --no-warn-dirty \
  --json \
  "${flake_ref}#nixUnitInventory.${system}" \
  >"$inventory_file" \
  2>"$inventory_stderr"; then
  sanitize_stderr_file "$inventory_stderr"
  fail "nix-unit inventory ($system): evaluation failed" || true
  exit 1
fi
if ! jq -e '
  type == "object"
  and ((keys | sort) == ["caseNames", "jobNames"])
  and (.caseNames | type == "array" and length > 0
    and all(.[]; type == "string" and length > 0))
  and (.jobNames | type == "array" and length > 0
    and all(.[]; type == "string" and length > 0))
  and (.caseNames == (.caseNames | sort | unique))
  and (.jobNames == (.jobNames | sort | unique))
' "$inventory_file" >/dev/null; then
  fail "nix-unit inventory ($system): output was not an object with non-empty caseNames and jobNames arrays" || true
  exit 1
fi

log "--> nix-eval-jobs --no-instantiate --flake ${flake_label}#nixUnitJobs.${system} --workers $workers --max-memory-size $memory_mb"
if nix-eval-jobs \
  --no-instantiate \
  --flake "${flake_ref}#nixUnitJobs.${system}" \
  --workers "$workers" \
  --max-memory-size "$memory_mb" \
  --show-trace >"$result_file" 2>"$tool_stderr"; then
  tool_status=0
else
  tool_status=$?
fi

if [ ! -s "$result_file" ] || ! jq -s -e '
  length > 0
  and all(.[]; type == "object"
    and ((.attr? != null) or (.attrPath? != null)))
' "$result_file" >/dev/null; then
  emit_sanitized_tool_stderr
  fail "nix-eval-jobs returned no valid JSON-lines attribute results" || true
  exit 1
fi

failures=()
failures_file="$result_dir/failures"
if ! jq -r -s '
    .[]
    | select(type == "object" and (.error? != null))
    | (.attr // ((.attrPath // []) | join("."))) as $attr
    | (
        .error
        | tostring
        | split("\n")
        | map(select(length > 0))
      ) as $error_lines
    | (
        $error_lines
        | map(
            select(
              test("^[[:space:]]*FAIL [^:]+: ")
              and (contains("${") | not)
            )
            | sub("^[[:space:]]+"; "")
          )
      ) as $fail_lines
    | if ($fail_lines | length) == 0 then
        "\($attr)\t\(
          if ($error_lines | length) == 0 then
            "evaluation failed without diagnostic"
          else
            ($error_lines | join(" ; "))
          end
        )"
      else
        $fail_lines[] | "\($attr)\t\(.)"
      end
  ' "$result_file" >"$failures_file"; then
  emit_sanitized_tool_stderr
  fail "could not parse nix-eval-jobs attribute failures" || true
  exit 1
fi
mapfile -t failures <"$failures_file"
result_count=$(jq -s 'length' "$result_file")

expected_result_attrs_unsorted="$result_dir/expected-result-attrs.unsorted"
expected_result_attrs_file="$result_dir/expected-result-attrs"
actual_result_attrs_unsorted="$result_dir/actual-result-attrs.unsorted"
actual_result_attrs_file="$result_dir/actual-result-attrs"
if ! jq -r '.jobNames[]' "$inventory_file" \
  >"$expected_result_attrs_unsorted"; then
  fail "could not read expected Nix-unit job names" || true
  exit 1
fi
if ! sort "$expected_result_attrs_unsorted" >"$expected_result_attrs_file"; then
  fail "could not sort expected Nix-unit result attributes" || true
  exit 1
fi
if ! jq -r -s '
    .[]
    | select(type == "object")
    | (.attr // ((.attrPath // []) | join(".")))
    | select(. != "")
  ' "$result_file" >"$actual_result_attrs_unsorted"; then
  emit_sanitized_tool_stderr
  fail "could not parse nix-eval-jobs result attributes" || true
  exit 1
fi
if ! sort "$actual_result_attrs_unsorted" >"$actual_result_attrs_file"; then
  fail "could not sort nix-eval-jobs result attributes" || true
  exit 1
fi
missing_result_attrs_file="$result_dir/missing-result-attrs"
unexpected_result_attrs_file="$result_dir/unexpected-result-attrs"
if ! comm -23 \
  "$expected_result_attrs_file" \
  "$actual_result_attrs_file" \
  >"$missing_result_attrs_file"; then
  fail "could not compare expected and evaluated Nix-unit result attributes" || true
  exit 1
fi
if ! comm -13 \
  "$expected_result_attrs_file" \
  "$actual_result_attrs_file" \
  >"$unexpected_result_attrs_file"; then
  fail "could not compare evaluated and expected Nix-unit result attributes" || true
  exit 1
fi
missing_result_attrs=()
unexpected_result_attrs=()
mapfile -t missing_result_attrs <"$missing_result_attrs_file"
mapfile -t unexpected_result_attrs <"$unexpected_result_attrs_file"
result_attrs_ok=1
if [ "${#missing_result_attrs[@]}" -ne 0 ] \
  || [ "${#unexpected_result_attrs[@]}" -ne 0 ]; then
  log "  FAIL: nix-unit result attributes differ from the locked file-job inventory"
  for attr in "${missing_result_attrs[@]}"; do
    log "    missing result attribute: $attr"
  done
  for attr in "${unexpected_result_attrs[@]}"; do
    log "    unexpected result attribute: $attr"
  done
  result_attrs_ok=0
fi

common_pin_file="$ROOT/tests/unit/nix/pinned/common.txt"
system_pin_file="$ROOT/tests/unit/nix/pinned/$system.txt"
if [ ! -f "$common_pin_file" ] || [ ! -f "$system_pin_file" ]; then
  missing_pin_files=()
  [ -f "$common_pin_file" ] || missing_pin_files+=("common.txt")
  [ -f "$system_pin_file" ] || missing_pin_files+=("$system.txt")
  fail "nix-unit case-presence pin files are missing for $system (${missing_pin_files[*]}); run make nix-unit-pin" || true
  exit 1
fi
expected_cases_unsorted="$result_dir/expected-cases.unsorted"
expected_cases_file="$result_dir/expected-cases"
actual_cases_file="$result_dir/actual-cases"
if ! awk '!/^#/ && NF { print $1 }' "$common_pin_file" "$system_pin_file" \
  >"$expected_cases_unsorted"; then
  fail "could not read pinned Nix-unit case names" || true
  exit 1
fi
if ! sort -u "$expected_cases_unsorted" >"$expected_cases_file"; then
  fail "could not sort pinned Nix-unit case names" || true
  exit 1
fi
inventory_cases_unsorted="$result_dir/inventory-cases.unsorted"
if ! jq -r '.caseNames[]' "$inventory_file" >"$inventory_cases_unsorted"; then
  fail "could not parse evaluated Nix-unit case names" || true
  exit 1
fi
if ! sort -u "$inventory_cases_unsorted" >"$actual_cases_file"; then
  fail "could not sort evaluated Nix-unit case names" || true
  exit 1
fi
missing_cases=()
unexpected_cases=()
missing_cases_file="$result_dir/missing-cases"
unexpected_cases_file="$result_dir/unexpected-cases"
if ! comm -23 "$expected_cases_file" "$actual_cases_file" >"$missing_cases_file"; then
  fail "could not compare pinned and evaluated Nix-unit cases" || true
  exit 1
fi
if ! comm -13 "$expected_cases_file" "$actual_cases_file" >"$unexpected_cases_file"; then
  fail "could not compare evaluated and pinned Nix-unit cases" || true
  exit 1
fi
mapfile -t missing_cases <"$missing_cases_file"
mapfile -t unexpected_cases <"$unexpected_cases_file"
case_names_ok=1
if [ "${#missing_cases[@]}" -ne 0 ] \
  || [ "${#unexpected_cases[@]}" -ne 0 ]; then
  log "  FAIL: nix-unit case inventory differs from pins for $system; run make nix-unit-pin"
  for case_name in "${missing_cases[@]}"; do
    log "    missing inventory case: $case_name"
  done
  for case_name in "${unexpected_cases[@]}"; do
    log "    unexpected inventory case: $case_name"
  done
  case_names_ok=0
fi

for failure in "${failures[@]}"; do
  sanitize_observable_line "$failure"
  failure=$sanitized_line
  failure_attr=${failure%%$'\t'*}
  failure_line=${failure#*$'\t'}
  printf '%s\n' \
    "  FAIL: nix-unit attribute $failure_attr: $failure_line" >&2
done
if [ "$tool_status" -ne 0 ]; then
  if [ "${#failures[@]}" -eq 0 ]; then
    emit_sanitized_tool_stderr
  fi
  log "  FAIL: nix-eval-jobs exited with status $tool_status"
fi
if [ "${#failures[@]}" -ne 0 ] \
  || [ "$tool_status" -ne 0 ] \
  || [ "$result_attrs_ok" -ne 1 ] \
  || [ "$case_names_ok" -ne 1 ]; then
  fail "nix-unit corpus ($system): ${#failures[@]} failure diagnostic(s) across $result_count results" || true
  exit 1
fi

nix_unit_command_succeeded=1
for leaf in "${nix_unit_baseline_leaves[@]}"; do
  if ! publish_manifest_fragment "$leaf" passed; then
    printf '%s\n' \
      "test-nix-unit: required execution-manifest fragment publication failed after successful surface '$leaf'; evidence is incomplete; retry the target." \
      >&2
    exit 1
  fi
done
nix_unit_surface=
log "test-nix-unit OK ($result_count attributes, $workers workers, ${memory_mb}MiB per worker; duration: $((SECONDS - suite_started))s)"
