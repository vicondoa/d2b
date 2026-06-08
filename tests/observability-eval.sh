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
OBS_DASHBOARD_DIR="$ROOT/nixos-modules/components/observability/dashboards"

PASS=0
FAIL=0
SKIP=0
AUTO_OBS_READY=false
RULES_STORE_PATH=""
HOST_ALLOY_STORE_PATH=""
POSTPASS_REALIZED=0

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

extract_dashboard_promql_exprs() {
  jq -r '.. | .expr? // empty' "$@" 2>/dev/null | sed '/^$/d'
}

extract_prometheus_rule_exprs() {
  jq -r '.groups[]?.rules[]?.expr // empty' "$1" 2>/dev/null | sed '/^$/d'
}

extract_rule_alert_names() {
  if jq -e '.groups[]?.rules[]?.alert // empty' "$1" >/dev/null 2>&1; then
    jq -r '.groups[]?.rules[]?.alert // empty' "$1" 2>/dev/null | LC_ALL=C sort -u
  fi
}

extract_dashboard_datasource_uids() {
  jq -r '.. | .datasource? | objects | .uid? // empty' "$@" 2>/dev/null | sed '/^$/d' | LC_ALL=C sort -u
}

extract_metric_tokens() {
  grep -oE '\b(nixling_[a-zA-Z0-9_:]+|node_[a-zA-Z0-9_:]+|systemd_unit_state|loki_[a-zA-Z0-9_:]+|tempo_[a-zA-Z0-9_:]+|prometheus_[a-zA-Z0-9_:]+|up)\b' \
    | LC_ALL=C sort -u
}

extract_up_job_refs() {
  {
    for file in "$@"; do
      sed -nE 's/.*up\{[^}]*job="([^"]+)"[^}]*\}.*/\1/p' "$file"
    done
  } | sed '/^$/d; /^\$/d' | LC_ALL=C sort -u
}

metric_reference_category() {
  local metric="$1"
  case "$metric" in
    up | ALERTS | ALERTS_FOR_STATE | absent_over_time) return 1 ;;
    alloy_* | prometheus_* | go_* | process_* | promhttp_* | scrape_* | target_info)
      printf '%s\n' 'control-plane'
      ;;
    loki_* )
      printf '%s\n' 'logs'
      ;;
    tempo_* | traces_* )
      printf '%s\n' 'traces'
      ;;
    node_* )
      printf '%s\n' 'guest'
      ;;
    nixling_* )
      printf '%s\n' 'nixling'
      ;;
    *)
      return 1
      ;;
  esac
}

realize_postpass_outputs() {
  local -a targets=()

  if [ "$POSTPASS_REALIZED" -eq 1 ]; then
    return 0
  fi

  [ -n "$HOST_ALLOY_STORE_PATH" ] && targets+=("$HOST_ALLOY_STORE_PATH")
  [ -n "$RULES_STORE_PATH" ] && targets+=("$RULES_STORE_PATH")

  if [ "${#targets[@]}" -eq 0 ]; then
    fail_case 'observability post-pass: missing Alloy/rules store paths from the batched eval output'
    return 1
  fi

  if ! nix-store --realise "${targets[@]}" >/dev/null; then
    fail_case 'observability post-pass: nix-store --realise failed for Alloy/rules outputs'
    return 1
  fi

  [ -n "$HOST_ALLOY_STORE_PATH" ] && [ ! -e "$HOST_ALLOY_STORE_PATH" ] && {
    fail_case 'observability post-pass: realized host Alloy config path is missing'
    return 1
  }
  [ -n "$RULES_STORE_PATH" ] && [ ! -e "$RULES_STORE_PATH" ] && {
    fail_case 'observability post-pass: realized Prometheus rules path is missing'
    return 1
  }

  POSTPASS_REALIZED=1
}

handle_obs_rules_promtool() {
  local promtool_out promtool_err

  require_success_case_json 'obs-rules-promtool' "$1" || return 1
  realize_postpass_outputs || return 1

  if ! command -v promtool >/dev/null 2>&1; then
    skip_case 'obs-rules-promtool: promtool not present in PATH (CHANGELOG Test-H note tracks the clean skip)'
    return 2
  fi

  promtool_out="$SCRATCH/obs-rules-promtool.stdout"
  promtool_err="$SCRATCH/obs-rules-promtool.stderr"
  if ! promtool check rules "$RULES_STORE_PATH" > "$promtool_out" 2> "$promtool_err"; then
    if [ -s "$promtool_out" ]; then
      log '    --- promtool (tail) ---'
      tail -15 "$promtool_out" | sed 's/^/      /' >&2
    fi
    show_stderr_tail "$promtool_err"
    fail_case 'obs-rules-promtool: promtool check rules failed'
    return 1
  fi

  pass_case 'obs-rules-promtool'
}

handle_obs_metric_references() {
  local dashboard_exprs rules_exprs metrics_file concrete_up_jobs metric category
  local -a unknown_metrics=()

  require_success_case_json 'obs-metric-references' "$1" || return 1
  realize_postpass_outputs || return 1

  dashboard_exprs="$SCRATCH/obs-metric-references.dashboard.promql"
  rules_exprs="$SCRATCH/obs-metric-references.rules.promql"
  metrics_file="$SCRATCH/obs-metric-references.metrics"

  extract_dashboard_promql_exprs "$OBS_DASHBOARD_DIR"/*.json > "$dashboard_exprs" || {
    fail_case 'obs-metric-references: could not extract dashboard PromQL expressions'
    return 1
  }

  extract_prometheus_rule_exprs "$RULES_STORE_PATH" > "$rules_exprs" || {
    fail_case 'obs-metric-references: could not extract alert PromQL expressions'
    return 1
  }

  cat "$dashboard_exprs" "$rules_exprs" | extract_metric_tokens > "$metrics_file" || {
    fail_case 'obs-metric-references: could not extract metric tokens from dashboard/rule PromQL'
    return 1
  }

  assert_ge "$(wc -l < "$metrics_file" | tr -d ' ')" '1' 'obs-metric-references: extracted metric references from PromQL' || return 1

  while IFS= read -r metric; do
    [ -n "$metric" ] || continue
    [ "$metric" = 'up' ] && continue
    category=$(metric_reference_category "$metric" || true)
    [ -z "$category" ] && unknown_metrics+=("$metric")
  done < "$metrics_file"

  if [ "${#unknown_metrics[@]}" -gt 0 ]; then
    fail_case "obs-metric-references: unresolved metric refs: ${unknown_metrics[*]}"
    return 1
  fi

  concrete_up_jobs=$(extract_up_job_refs "$dashboard_exprs" "$rules_exprs" | sed '/^\$job$/d')
  while IFS= read -r metric; do
    [ -n "$metric" ] || continue
    case "$metric" in
      alloy|grafana|loki|nixling-ch-exporter|nixling-vm-telemetry|prometheus|tempo) ;;
      *)
        fail_case "obs-metric-references: concrete up{job=...} ref '$metric' is outside the known scrape-job set"
        return 1
        ;;
    esac
  done <<< "$concrete_up_jobs"
  assert_ge "$(printf '%s\n' "$concrete_up_jobs" | sed '/^$/d' | wc -l | tr -d ' ')" '1' \
    'obs-metric-references: concrete up{job=...} refs stay on known scrape jobs' || return 1

  pass_case 'obs-metric-references'
}

handle_obs_scrape_job_stability() {
  local host_jobs

  require_success_case_json 'obs-scrape-job-stability' "$1" || return 1
  realize_postpass_outputs || return 1

  host_jobs=$(grep -oE 'job_name[[:space:]]*=[[:space:]]*"[^"]+"' "$HOST_ALLOY_STORE_PATH" | sed -E 's/.*"([^"]+)"/\1/' | LC_ALL=C sort -u)
  assert_lines_set_eq "$host_jobs" $'nixling-ch-exporter\nsystemd-units' \
    'obs-scrape-job-stability: host Alloy scrape job exact-set' || return 1

  pass_case 'obs-scrape-job-stability'
}

handle_obs_stability() {
  local dashboard_uids dashboard_datasource_uids alert_names host_jobs

  require_success_case_json 'obs-stability' "$1" || return 1
  realize_postpass_outputs || return 1

  dashboard_uids=$(jq -r '.uid' "$OBS_DASHBOARD_DIR"/*.json | LC_ALL=C sort -u) || return 1
  dashboard_datasource_uids=$(extract_dashboard_datasource_uids "$OBS_DASHBOARD_DIR"/*.json) || return 1
  alert_names=$(extract_rule_alert_names "$RULES_STORE_PATH")
  host_jobs=$(grep -oE 'job_name[[:space:]]*=[[:space:]]*"[^"]+"' "$HOST_ALLOY_STORE_PATH" | sed -E 's/.*"([^"]+)"/\1/' | LC_ALL=C sort -u)

  assert_lines_set_eq "$dashboard_uids" $'lifecycle-traces\nlogs\nnixling-overview\nobs-vm-health\nper-vm-store\nvm-resources' \
    'obs-stability: dashboard UID exact-set' || return 1
  assert_lines_set_eq "$dashboard_datasource_uids" $'loki\nprometheus\ntempo' \
    'obs-stability: dashboard datasource UID exact-set' || return 1
  assert_lines_set_eq "$alert_names" $'NixlingCHAPISocketMissing\nNixlingGuestTelemetryMissing\nNixlingNetVMDownWithRunningWorkloads\nNixlingObsVMStackUnhealthy\nNixlingObsVMUnreachableFromHost\nNixlingStoreSyncFailure\nNixlingVMDown\nNixlingVsockRelayDown' \
    'obs-stability: alert-rule exact-set' || return 1
  assert_lines_set_eq "$host_jobs" $'nixling-ch-exporter\nsystemd-units' \
    'obs-stability: host scrape-job exact-set' || return 1

  pass_case 'obs-stability'
}

handle_case() {
  local case_name="$1" case_json="$2" kind

  case "$case_name" in
    obs-enabled-defaults)
      if [ "$AUTO_OBS_READY" != 'true' ]; then
        skip_case 'obs-enabled-defaults: TODO post-integration — auto-obs-vm has not materialized sys-obs-stack + obs env in this worktree'
        return 2
      fi
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
      if [ "$AUTO_OBS_READY" != 'true' ]; then
        skip_case 'obs-reserved-prefix-exempt: TODO post-integration — auto-obs-vm has not landed, so sys-obs-stack is not materialized in this worktree'
        return 2
      fi
      require_success_case_json "$case_name" "$case_json" || return 1
      pass_case "$case_name"
      ;;
    obs-rules-promtool)
      handle_obs_rules_promtool "$case_json"
      ;;
    obs-metric-references)
      handle_obs_metric_references "$case_json"
      ;;
    obs-scrape-job-stability)
      handle_obs_scrape_job_stability "$case_json"
      ;;
    obs-stability)
      handle_obs_stability "$case_json"
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
    skip_case 'observability-eval: microvm guest surface absent in daemon-only config'
    log "==> observability-eval: ${PASS} passed, ${FAIL} failed, ${SKIP} skipped"
    exit 0
  fi
  show_stderr_tail "$batch_stderr"
  fail_case 'observability-eval: batched nix-instantiate failed' || true
  FAIL=$((FAIL + 1))
  log "==> observability-eval: ${PASS} passed, ${FAIL} failed, ${SKIP} skipped"
  exit 1
fi

AUTO_OBS_READY=$(jq -r '(."obs-enabled-defaults".extracted.hasSysObsStack // false) and (."obs-enabled-defaults".extracted.hasObsEnv // false)' "$BATCH_JSON")
RULES_STORE_PATH=$(jq -r '."obs-alerting-surface".aux.rulesPath // ."obs-rules-promtool".aux.rulesPath // ."obs-stability".aux.rulesPath // empty' "$BATCH_JSON")
HOST_ALLOY_STORE_PATH=$(jq -r '."obs-scrape-job-stability".aux.hostAlloyConfigPath // ."obs-stability".aux.hostAlloyConfigPath // empty' "$BATCH_JSON")

while IFS= read -r entry; do
  case_name=$(printf '%s\n' "$entry" | jq -r '.key')
  case_json=$(printf '%s\n' "$entry" | jq -c '.value')
  run_case handle_case "$case_name" "$case_json"
done < <(jq -c 'to_entries[]' "$BATCH_JSON")

log "==> observability-eval: ${PASS} passed, ${FAIL} failed, ${SKIP} skipped"
[ "$FAIL" -eq 0 ]
