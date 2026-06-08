#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=lib.sh
. "$HERE/lib.sh"

log "==> tests/l1c-privilege-oracle.sh"

if [ "$(id -u)" -ne 0 ]; then
  log "SKIP: l1c privilege oracle requires root on a self-hosted L1c runner"
  exit 0
fi

if [ "${NIXLING_L1C_SELF_HOSTED:-0}" != 1 ]; then
  log "SKIP: set NIXLING_L1C_SELF_HOSTED=1 on the self-hosted L1c runner to enable the privilege oracle"
  exit 0
fi

oracle_json=${NIXLING_L1C_PRIVILEGE_ORACLE_JSON:-}
if [ -z "$oracle_json" ] || [ ! -f "$oracle_json" ]; then
  log "SKIP: set NIXLING_L1C_PRIVILEGE_ORACLE_JSON to the runner-local broker oracle table"
  exit 0
fi

broker_pid=${NIXLING_L1C_BROKER_PID:-}
if [ -z "$broker_pid" ] && command -v systemctl >/dev/null 2>&1; then
  broker_pid=$(systemctl show -p MainPID --value nixling-priv-broker.service 2>/dev/null || true)
fi
if [ -z "$broker_pid" ] || [ "$broker_pid" = 0 ]; then
  broker_pid=$(ps -eo pid=,comm= | awk '$2 == "nixling-priv-broker" {print $1; exit}')
fi
if [ -z "$broker_pid" ] || [ ! -d "/proc/$broker_pid" ]; then
  fail "l1c privilege oracle could not locate a live nixling-priv-broker process"
  exit 1
fi

status_file="/proc/$broker_pid/status"
cgroup_file="/proc/$broker_pid/cgroup"
[ -r "$status_file" ] || { fail "cannot read $status_file"; exit 1; }
[ -r "$cgroup_file" ] || { fail "cannot read $cgroup_file"; exit 1; }

actual_uid=$(awk '/^Uid:/ {print $2}' "$status_file")
actual_gid=$(awk '/^Gid:/ {print $2}' "$status_file")
actual_cap_eff=$(awk '/^CapEff:/ {print $2}' "$status_file")
actual_cap_bnd=$(awk '/^CapBnd:/ {print $2}' "$status_file")
actual_cap_amb=$(awk '/^CapAmb:/ {print $2}' "$status_file")
actual_nonewprivs=$(awk '/^NoNewPrivs:/ {print $2}' "$status_file")
actual_seccomp=$(awk '/^Seccomp:/ {print $2}' "$status_file")
actual_cgroup=$(awk -F: '$1 == "0" {print $3; found=1} {last=$3} END {if (!found) print last}' "$cgroup_file")

compare_exact() {
  local field="$1" actual="$2" expected="$3" op="$4"
  if [ "$actual" != "$expected" ]; then
    fail "l1c privilege oracle mismatch for $op ($field): expected '$expected', got '$actual'"
    exit 1
  fi
}

compare_regex() {
  local field="$1" actual="$2" expected_regex="$3" op="$4"
  if [[ ! "$actual" =~ $expected_regex ]]; then
    fail "l1c privilege oracle mismatch for $op ($field): expected regex '$expected_regex', got '$actual'"
    exit 1
  fi
}

read_oracle_exact() {
  local key="$1"
  jq -r --arg key "$key" '.[$key] // empty' "$oracle_json"
}

read_oracle_regex() {
  local namespace_name="$1"
  jq -r --arg ns "$namespace_name" '.namespaces[$ns] // empty' "$oracle_json"
}

expected_uid=$(read_oracle_exact uid)
expected_gid=$(read_oracle_exact gid)
expected_cap_eff=$(read_oracle_exact capEff)
expected_cap_bnd=$(read_oracle_exact capBnd)
expected_cap_amb=$(read_oracle_exact capAmb)
expected_nonewprivs=$(read_oracle_exact noNewPrivs)
expected_seccomp=$(read_oracle_exact seccomp)
expected_cgroup_regex=$(read_oracle_exact cgroupPathRegex)

for required in \
  expected_uid \
  expected_gid \
  expected_cap_eff \
  expected_cap_bnd \
  expected_cap_amb \
  expected_nonewprivs \
  expected_seccomp \
  expected_cgroup_regex; do
  if [ -z "${!required}" ]; then
    fail "l1c privilege oracle JSON is missing required field ${required#expected_}"
    exit 1
  fi
done

operations=$(jq -r '.brokerOperations[].operation' "$ROOT/docs/reference/schemas/v2/privileges.json")
[ -n "$operations" ] || { fail "no broker operations found in docs/reference/schemas/v2/privileges.json"; exit 1; }

validated=0
for op in $operations; do
  compare_exact uid "$actual_uid" "$expected_uid" "$op"
  compare_exact gid "$actual_gid" "$expected_gid" "$op"
  compare_exact CapEff "$actual_cap_eff" "$expected_cap_eff" "$op"
  compare_exact CapBnd "$actual_cap_bnd" "$expected_cap_bnd" "$op"
  compare_exact CapAmb "$actual_cap_amb" "$expected_cap_amb" "$op"
  compare_exact NoNewPrivs "$actual_nonewprivs" "$expected_nonewprivs" "$op"
  compare_exact Seccomp "$actual_seccomp" "$expected_seccomp" "$op"
  compare_regex cgroup "$actual_cgroup" "$expected_cgroup_regex" "$op"

  for ns_name in mnt pid net ipc uts user cgroup time time_for_children; do
    expected_ns=$(read_oracle_regex "$ns_name")
    if [ -n "$expected_ns" ]; then
      actual_ns=$(readlink "/proc/$broker_pid/ns/$ns_name")
      compare_regex "ns:$ns_name" "$actual_ns" "$expected_ns" "$op"
    fi
  done
  validated=$((validated + 1))
done

ok "validated broker uid/gid/cap/seccomp/cgroup/namespace oracle across $validated broker operations"
