#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

log "==> tests/performance-budgets.sh"

if [ "${NIXLING_PERF_STABLE:-0}" != 1 ]; then
  log "SKIP: set NIXLING_PERF_STABLE=1 on a pinned self-hosted runner to enforce performance budgets"
  exit 0
fi

if [ -z "${NIXLING_PERF_IN_NIX_SHELL:-}" ] && ! command -v python3 >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "performance budgets require python3 (or nix to provide it) once NIXLING_PERF_STABLE=1 is set"
    exit 1
  fi
  export NIXLING_PERF_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" nixpkgs#python3 --command bash "$0" "$@"
fi

DAEMON_HELLO_BUDGET_MS=${NIXLING_PERF_DAEMON_HELLO_BUDGET_MS:-200}
BROKER_VALIDATE_P99_BUDGET_MS=${NIXLING_PERF_BROKER_VALIDATE_P99_BUDGET_MS:-25}
VM_START_DRY_RUN_BUDGET_MS=${NIXLING_PERF_VM_START_DRY_RUN_BUDGET_MS:-100}
MARGIN_PERCENT=20

scratch=$(nl_mktemp .performance-budgets.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""
bundle_root=$(nl_cli_smoke_bundle_tree)
cli_bin=$(nl_cli_native_bin)
daemon_bin=$(nl_daemon_native_bin)
broker_bin=$(nl_cargo_bin_path broker nixling-priv-broker)
if [ ! -x "$broker_bin" ]; then
  nl_cli_toolchain_shell "cd '$ROOT/packages/nixling-priv-broker' && CARGO_TARGET_DIR='$(nl_cargo_target_dir broker)' cargo build -q --manifest-path '$ROOT/packages/nixling-priv-broker/Cargo.toml' -p nixling-priv-broker"
fi

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

median_of_three() {
  printf '%s\n' "$1" "$2" "$3" | sort -n | sed -n '2p'
}

measure_daemon_cold_start_ms() {
  local socket_path="$scratch/daemon/public.sock"
  local state_lock="$scratch/daemon/daemon.lock"
  local locks_dir="$scratch/daemon/locks"
  local config_json="$scratch/daemon/config.json"
  local sample1 sample2 sample3
  mkdir -p "$scratch/daemon"
  chmod 0755 "$scratch/daemon"
  cat >"$config_json" <<EOF
{
  "publicSocketPath": "$socket_path",
  "brokerSocketPath": "$scratch/daemon/priv.sock",
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

  measure_once() {
    local start_ms end_ms daemon_pid attempts
    rm -f "$socket_path" "$state_lock"
    mkdir -p "$locks_dir"
    start_ms=$(nl_now_ms)
    (
      export NIXLINGD_TEST_PEER_UID=60003
      export NIXLINGD_TEST_PEER_GID=60003
      export NIXLINGD_TEST_PEER_USERNAME=launcher-user
      export NIXLINGD_TEST_PEER_GROUPS=wheel
      "$daemon_bin" serve \
        --config "$config_json" \
        --test-listen-on "$socket_path" \
        --state-lock "$state_lock" \
        --locks-dir "$locks_dir" \
        --once \
        --allow-unprivileged-runtime-dir \
        --no-drop-privileges
    ) >"$scratch/daemon/serve.log" 2>&1 &
    daemon_pid=$!
    add_cleanup "kill $daemon_pid >/dev/null 2>&1 || true"
    attempts=0
    while [ "$attempts" -lt 300 ]; do
      if "$daemon_bin" test-client --socket "$socket_path" --frame-json '{"type":"hello","clientVersion":">=0.4.0, <0.5.0","supportedFeatures":[]}' >/dev/null 2>&1; then
        end_ms=$(nl_now_ms)
        wait "$daemon_pid"
        printf '%s\n' $((end_ms - start_ms))
        return 0
      fi
      attempts=$((attempts + 1))
      sleep 0.01
    done
    fail "daemon cold-start measurement timed out waiting for the first hello response"
    exit 1
  }

  sample1=$(measure_once)
  sample2=$(measure_once)
  sample3=$(measure_once)
  median_of_three "$sample1" "$sample2" "$sample3"
}

measure_broker_validate_p99_ms() {
  local socket_path="$scratch/broker/priv.sock"
  local audit_dir="$scratch/broker/audit"
  local broker_pid
  mkdir -p "$scratch/broker" "$audit_dir"
  rm -f "$socket_path"
  (
    "$broker_bin" serve \
      --socket-path "$socket_path" \
      --audit-dir "$audit_dir" \
      --audit-retention-days 0 \
      --bundle-path "$bundle_root/vms.json" \
      --nixlingd-uid "$(id -u)" \
      --nixlingd-gid "$(id -g)" \
      --test-mode
  ) >"$scratch/broker/serve.log" 2>&1 &
  broker_pid=$!
  add_cleanup "kill $broker_pid >/dev/null 2>&1 || true"
  wait_for_socket "$socket_path"
  python3 - "$socket_path" "$(id -u)" <<'PY'
import json
import socket
import struct
import sys
import time

socket_path = sys.argv[1]
peer_uid = int(sys.argv[2])
samples = []
for _ in range(100):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
    s.connect(socket_path)
    payload = json.dumps({
        "request": {"kind": "ValidateBundle"},
        "testPeerUid": peer_uid,
    }).encode()
    frame = struct.pack("<I", len(payload)) + payload
    start = time.perf_counter_ns()
    s.sendall(frame)
    response = s.recv(1024 * 1024)
    end = time.perf_counter_ns()
    s.close()
    if len(response) < 4:
        raise SystemExit("broker response shorter than length prefix")
    declared = struct.unpack("<I", response[:4])[0]
    body = response[4:]
    if declared != len(body):
        raise SystemExit("broker response length prefix mismatch")
    parsed = json.loads(body.decode())
    kind = parsed.get("kind")
    if kind not in {"ValidateBundleOk", "ValidateBundle"}:
        raise SystemExit(f"unexpected broker response kind: {kind!r}")
    samples.append((end - start + 999_999) // 1_000_000)

samples.sort()
index = max(0, min(len(samples) - 1, ((99 * len(samples) + 99) // 100) - 1))
print(samples[index])
PY
  kill "$broker_pid"
  wait "$broker_pid" || true
}

measure_vm_start_dry_run_ms() {
  local sample1 sample2 sample3
  measure_once() {
    local start_ms end_ms
    start_ms=$(nl_now_ms)
    NIXLING_MANIFEST_PATH="$bundle_root/vms.json" \
    NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
      "$cli_bin" vm start corp-vm --dry-run --json >/dev/null
    end_ms=$(nl_now_ms)
    printf '%s\n' $((end_ms - start_ms))
  }

  sample1=$(measure_once)
  sample2=$(measure_once)
  sample3=$(measure_once)
  median_of_three "$sample1" "$sample2" "$sample3"
}

assert_budget() {
  local label="$1" actual_ms="$2" budget_ms="$3"
  local allowed_ms
  allowed_ms=$(( (budget_ms * (100 + MARGIN_PERCENT) + 99) / 100 ))
  if [ "$actual_ms" -le "$allowed_ms" ]; then
    ok "$label: ${actual_ms}ms <= ${allowed_ms}ms threshold (budget ${budget_ms}ms +${MARGIN_PERCENT}%)"
  else
    fail "$label regression: ${actual_ms}ms exceeds ${allowed_ms}ms threshold (budget ${budget_ms}ms +${MARGIN_PERCENT}%)"
    exit 1
  fi
}

daemon_hello_ms=$(measure_daemon_cold_start_ms)
broker_validate_p99_ms=$(measure_broker_validate_p99_ms)
vm_start_dry_run_ms=$(measure_vm_start_dry_run_ms)

assert_budget "daemon cold start → first Hello" "$daemon_hello_ms" "$DAEMON_HELLO_BUDGET_MS"
assert_budget "broker ValidateBundle p99" "$broker_validate_p99_ms" "$BROKER_VALIDATE_P99_BUDGET_MS"
assert_budget "nixling vm start --dry-run wall time" "$vm_start_dry_run_ms" "$VM_START_DRY_RUN_BUDGET_MS"
