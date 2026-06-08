#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

log "==> tests/daemon-state-persistence.sh"
scratch=$(nl_mktemp .daemon-state-persistence.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""

runtime_root="$scratch/run"
state_dir="$scratch/state"
socket_path="$runtime_root/public.sock"
state_lock="$runtime_root/daemon.lock"
locks_dir="$runtime_root/locks"
config_json="$scratch/config.json"
report_json="$scratch/state-restore-report.json"
stop_response_json="$scratch/vm-stop-response.json"
pidfd_table_json="$state_dir/pidfd-table.json"
runtime_snapshot_json="$state_dir/corp-vm/runtime.ch.json"
mkdir -p "$runtime_root" "$state_dir"
chmod 0755 "$runtime_root"

daemon_bin=$(nl_daemon_native_bin)

cat >"$config_json" <<EOF
{
  "publicSocketPath": "$socket_path",
  "brokerSocketPath": "$scratch/priv.sock",
  "stateLockPath": "$state_lock",
  "locksDir": "$locks_dir",
  "daemonUser": "root",
  "daemonGroup": "root",
  "publicSocketGroup": "$(id -gn)",
  "launcherUsers": ["launcher-user"],
  "adminUsers": ["admin-user"],
  "serverVersion": "0.4.0",
  "acceptedClientVersionRange": ">=0.4.0, <0.5.0"
}
EOF

wait_for_socket() {
  local path="$1"
  local attempts=0
  while [ "$attempts" -lt 300 ]; do
    [ -S "$path" ] && return 0
    attempts=$((attempts + 1))
    sleep 0.1
  done
  fail "timed out waiting for socket: $path"
  return 1
}

STARTED_DAEMON_PID=
start_daemon() {
  local label="$1" once_mode="$2" report_path="$3"
  rm -f "$socket_path" "$state_lock"
  mkdir -p "$locks_dir"
  (
    export NIXLINGD_TEST_PEER_UID=60003
    export NIXLINGD_TEST_PEER_GID=60003
    export NIXLINGD_TEST_PEER_USERNAME=launcher-user
    export NIXLINGD_TEST_PEER_GROUPS=wheel
    cmd=(
      "$daemon_bin" serve
      --config "$config_json"
      --test-listen-on "$socket_path"
      --state-lock "$state_lock"
      --locks-dir "$locks_dir"
      --allow-unprivileged-runtime-dir
      --no-drop-privileges
    )
    if [ "$once_mode" = yes ]; then
      cmd+=(--once)
    fi
    if [ -n "$report_path" ]; then
      cmd+=(
        --daemon-state-dir "$state_dir"
        --test-state-restore-report "$report_path"
      )
    fi
    "${cmd[@]}"
  ) >"$scratch/$label.server.log" 2>&1 &
  STARTED_DAEMON_PID=$!
  add_cleanup "kill $STARTED_DAEMON_PID >/dev/null 2>&1 || true"
  wait_for_socket "$socket_path"
}

run_hello_and_vm_stop_once() {
  local client_output
  client_output=$("$daemon_bin" test-client \
    --socket "$socket_path" \
    --frame-json '{"type":"hello","clientVersion":">=0.4.0, <0.5.0","supportedFeatures":[]}' \
    --frame-json '{"type":"vmStop","vm":"corp-vm","apply":true,"json":true}')
  printf '%s\n' "$client_output" | sed -n '$p' >"$stop_response_json"
}

start_daemon initial no ""
first_pid=$STARTED_DAEMON_PID
kill "$first_pid"
wait "$first_pid" || true
rm -f "$socket_path" "$state_lock"

sleep 120 &
runner_pid=$!
add_cleanup "kill $runner_pid >/dev/null 2>&1 || true"
start_time_ticks=$(awk '{print $22}' "/proc/$runner_pid/stat")
mkdir -p "$state_dir/corp-vm"
jq -n \
  --arg vm "corp-vm" \
  --arg role "ch-runner" \
  --argjson pid "$runner_pid" \
  --argjson startTimeTicks "$start_time_ticks" \
  '{entries: [{vm: $vm, role: $role, pid: $pid, startTimeTicks: $startTimeTicks}]}' \
  >"$pidfd_table_json"
jq -n \
  --arg vm "corp-vm" \
  --arg roleId "ch" \
  --arg role "cloud-hypervisor" \
  --argjson pid "$runner_pid" \
  --argjson startTimeTicks "$start_time_ticks" \
  --arg snapshottedAt "2026-01-01T00:00:00Z" \
  '{vm: $vm, roleId: $roleId, role: $role, pid: $pid, startTimeTicks: $startTimeTicks, snapshottedAt: $snapshottedAt}' \
  >"$runtime_snapshot_json"

start_daemon restore yes "$report_json"
second_pid=$STARTED_DAEMON_PID
run_hello_and_vm_stop_once
wait "$second_pid"

if jq -e '.entries | length == 1 and .[0].vm == "corp-vm" and .[0].roleId == "ch" and .[0].outcome.outcome == "adopt"' "$report_json" >/dev/null 2>&1; then
  ok "daemon restart reconciles runtime snapshot report"
else
  cat "$report_json" >&2 || true
  fail "daemon state restore report did not adopt the persisted runtime snapshot"
fi

if jq -e '.type == "mutatingVerbResponse" and .verb == "vm stop" and .outcome == "applied" and .summary == "vm stop corp-vm: ch-runner terminated via pidfd_table after SIGTERM"' "$stop_response_json" >/dev/null 2>&1; then
  ok "daemon vm stop used restored pidfd-table entry"
else
  cat "$stop_response_json" >&2 || true
  fail "daemon vm stop did not use the restored pidfd-table entry"
fi

wait "$runner_pid" || true
if kill -0 "$runner_pid" >/dev/null 2>&1; then
  fail "restored runner pid is still alive after vm stop"
fi
ok "restored runner terminated"

if jq -e '.entries | length == 0' "$pidfd_table_json" >/dev/null 2>&1; then
  ok "pidfd-table snapshot cleared after stop"
else
  cat "$pidfd_table_json" >&2 || true
  fail "pidfd-table.json was not rewritten after stop"
fi
